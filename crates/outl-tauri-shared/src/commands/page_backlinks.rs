//! Resolving a page's backlinks without freezing the WebView.
//!
//! Split out of `commands::page` because it is not page navigation —
//! it is a concurrency problem with a specific, expensive history, and
//! every line of it is load-bearing: the three phases exist to keep an
//! `O(blocks)` walk off both the IPC thread and the workspace lock
//! (#179), and the lock order is an ABBA deadlock the moment it is
//! taken the other way round.
//!
//! The reply also carries the page's nested-pages rows (issue #275),
//! derived from the page list phase 1 already read — see
//! [`finish_lookup`].

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    build_backlink_index_from_disk, find_by_slug, list_pages, namespace_descendants,
    page_meta as page_meta_action, sort_backlinks, BacklinkIndex, PageMeta,
};
use outl_core::workspace::Workspace;
use parking_lot::Mutex;

use crate::state::{BacklinksReply, ERR_LOADING};

/// Resolve a page's backlinks off the Tauri IPC/main thread, from the
/// pre-computed index (built lazily from the `.md` projection on disk).
///
/// Three phases, none of which materialize the workspace: (1) a brief
/// workspace lock to read the page list + this page's meta (`list_pages`
/// reads page roots only, never block text); (2) an `O(blocks)` index
/// rebuild **from disk** when the slot is stale, holding no lock and
/// touching no `Workspace` — reading block text through
/// `Workspace::block_text` here would force a lazy-boot vault (#179) to
/// materialize the whole thing, the "opening the journal / pressing Esc
/// freezes for seconds" bug (crash backtrace `page_backlinks →
/// backlinks_for_page → block_text → materialize_text_from_log` on
/// `com.apple.main-thread`); (3) an `O(refs)` lookup under a brief lock.
/// Run off the IPC thread so the WebView stays responsive; the panel
/// fills in a beat later.
pub(crate) async fn compute_backlinks_offloaded(
    workspace: Arc<Mutex<Option<Workspace>>>,
    index: Option<Arc<Mutex<Option<BacklinkIndex>>>>,
    root: PathBuf,
    slug: String,
) -> Result<BacklinksReply, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Phase 1: page list + this page's meta, under a BRIEF workspace
        // lock. `list_pages` reads page roots only — it never materializes
        // block text, so this doesn't trip the #179 freeze.
        let (metas, meta) = {
            let guard = workspace.lock();
            let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
            let id = find_by_slug(ws, &slug).ok_or_else(|| format!("page not found: {slug}"))?;
            let meta = page_meta_action(ws, id).ok_or_else(|| format!("page not found: {slug}"))?;
            (list_pages(ws), meta)
        };
        let backlinks_order = outl_config::load().display.backlinks_order;

        let index = match index {
            Some(i) => i,
            None => {
                // Host without a cached slot: one-shot from-disk build.
                let idx = build_backlink_index_from_disk(&metas, &root);
                let guard = workspace.lock();
                let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
                return Ok(finish_lookup(ws, &idx, &meta, &metas, backlinks_order));
            }
        };
        // Phase 2: build the index FROM DISK when stale — reads the `.md`
        // projection, touches no `Workspace`, holds NO lock. This is what
        // keeps opening the journal / pressing Esc from freezing: the
        // O(blocks) work never materializes the vault and never blocks an
        // edit waiting on the workspace lock.
        //
        // Phase 3: O(refs) lookup under a brief lock (`for_page` reads the
        // page's own `template::` property; no block-text scan).
        //
        // **Workspace first, then the index — never the other way round.**
        // A commit holds the workspace lock for the whole of
        // `finish_in_page_with` and drops the cached index from inside it
        // (`invalidate_backlink_index`), so this thread taking the index
        // lock first and then waiting on the workspace is an ABBA deadlock
        // with no timeout: the app freezes until it is force-quit. A paste
        // is the reliable way in, because it commits twice (draft flush +
        // the paste) and each commit refreshes this panel.
        // See `tests/backlinks_commit_deadlock.rs`.
        //
        // Nothing ties the build to the lookup: a commit can run
        // `invalidate_backlink_index` in between and legitimately empty
        // the slot again. That is a retry, never a panic: a panic in this
        // blocking task surfaces as a join error the frontend ignores, and
        // the panel silently stops refreshing.
        for _ in 0..3 {
            if index.lock().is_none() {
                let fresh = build_backlink_index_from_disk(&metas, &root);
                let mut g = index.lock();
                if g.is_none() {
                    *g = Some(fresh);
                }
            }
            let guard = workspace.lock();
            let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
            let g = index.lock();
            if let Some(idx) = g.as_ref() {
                return Ok(finish_lookup(ws, idx, &meta, &metas, backlinks_order));
            }
            // Invalidated between the build and the lookup: drop both
            // locks and rebuild.
        }
        // Commits are invalidating faster than the cache refills; serve a
        // one-shot from-disk build directly, same as the host-without-a-slot
        // path above (marginally stale is fine, the next refresh re-caches).
        let idx = build_backlink_index_from_disk(&metas, &root);
        let guard = workspace.lock();
        let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
        Ok(finish_lookup(ws, &idx, &meta, &metas, backlinks_order))
    })
    .await
    .map_err(|e| format!("backlinks task join: {e}"))?
}

/// Look a page up in `index` and shape the reply. GUI rows render only
/// `source_block.tokens`, so each hit ships through `into_shallow` to
/// keep the subtree off the IPC wire.
///
/// Takes the already-locked `&Workspace` (`for_page` reads the page's
/// `template::` property) rather than the `Arc<Mutex<…>>`: the caller owns
/// the lock order, and the only safe one here is workspace → index.
/// How many namespace-only backlinks ride the wire per page.
///
/// The set is unbounded — a namespace with thousands of mentions is
/// normal on a real graph — and a client renders it as a collapsed
/// section, so the whole list was never going to be read. 50 is enough
/// to be useful when opened and small enough that the count, not the
/// payload, carries the scale.
pub(crate) const NAMESPACE_BACKLINK_CAP: usize = 50;

fn finish_lookup(
    ws: &Workspace,
    index: &BacklinkIndex,
    meta: &PageMeta,
    metas: &[PageMeta],
    backlinks_order: outl_config::BacklinksOrder,
) -> BacklinksReply {
    let split = index.for_page_split(ws, meta);
    let mut backlinks: Vec<_> = split.direct.into_iter().map(|b| b.into_shallow()).collect();
    sort_backlinks(&mut backlinks, backlinks_order.newest_first());

    // Sort before truncating, or the 50 that ship are whichever the
    // walk happened to reach first rather than the newest.
    let namespace_backlinks_total = split.namespaced.len();
    let mut namespace_backlinks: Vec<_> = split
        .namespaced
        .into_iter()
        .map(|b| b.into_shallow())
        .collect();
    sort_backlinks(&mut namespace_backlinks, backlinks_order.newest_first());
    namespace_backlinks.truncate(NAMESPACE_BACKLINK_CAP);

    BacklinksReply {
        backlinks,
        backlinks_order,
        namespace_backlinks,
        namespace_backlinks_total,
        // Derived from the page list phase 1 already read — the title
        // is where the `/` survives (the slug folds it), so the
        // namespace question is asked of titles. See
        // `outl_actions::namespace`.
        namespace_children: namespace_descendants(metas, &meta.title),
    }
}
