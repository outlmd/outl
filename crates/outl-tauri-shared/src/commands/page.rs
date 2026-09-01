//! Page / journal navigation command bodies.

use outl_actions::{
    build_backlink_index_from_disk, find_by_slug, journal_slug, journal_title, list_pages,
    next_journal_date, open_journal, open_or_create_by_name, open_or_create_by_ref, open_today,
    page_meta as page_meta_action, previous_journal_date, remove_page_projection,
    search_persons as action_search_persons, sort_backlinks, today, BacklinkIndex, PageKind,
    PageMeta,
};

use outl_md::index::WorkspaceIndex;
use outl_md::{BlockEntry, BlockIndex};

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::workspace::Workspace;
use parking_lot::Mutex;

use crate::helpers::{
    build_page_view, parse_date, parse_node_id, reproject_stale_md, with_ws, with_ws_mut,
};
use crate::host::AppHost;
use crate::state::{BacklinksReply, BlockHit, PageView, ERR_LOADING};

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
async fn compute_backlinks_offloaded(
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

        // Phase 2: build the index FROM DISK when stale — reads the `.md`
        // projection, touches no `Workspace`, holds NO lock. This is what
        // keeps opening the journal / pressing Esc from freezing: the
        // O(blocks) work never materializes the vault and never blocks an
        // edit waiting on the workspace lock.
        let index = match index {
            Some(i) => i,
            None => {
                // Host without a cached slot: one-shot from-disk build.
                let idx = build_backlink_index_from_disk(&metas, &root);
                let guard = workspace.lock();
                let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
                return Ok(finish_lookup(ws, &idx, &meta, backlinks_order));
            }
        };
        if index.lock().is_none() {
            let fresh = build_backlink_index_from_disk(&metas, &root);
            let mut g = index.lock();
            if g.is_none() {
                *g = Some(fresh);
            }
        }

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
        let guard = workspace.lock();
        let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
        let g = index.lock();
        let idx = g.as_ref().expect("index just built");
        Ok(finish_lookup(ws, idx, &meta, backlinks_order))
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
fn finish_lookup(
    ws: &Workspace,
    index: &BacklinkIndex,
    meta: &PageMeta,
    backlinks_order: outl_config::BacklinksOrder,
) -> BacklinksReply {
    let mut backlinks: Vec<_> = index
        .for_page(ws, meta)
        .into_iter()
        .map(|b| b.into_shallow())
        .collect();
    sort_backlinks(&mut backlinks, backlinks_order.newest_first());
    BacklinksReply {
        backlinks,
        backlinks_order,
    }
}

pub fn list_all_pages<S: AppHost>(state: &S) -> Result<Vec<PageMeta>, String> {
    with_ws(state, |ws| Ok(list_pages(ws)))
}

/// Filter known pages by a case-insensitive substring match against
/// title + slug. Powers the quick switcher and the `[[…]]` ref
/// suggester.
///
/// Ranking, weakest filter to strongest: title or slug
/// `starts_with(query)` outranks an interior match; exact equality
/// (case-insensitive) ranks above prefix. Capped at 25 so the floating
/// list stays cheap to render. An empty query returns the most recent
/// pages (whatever order `list_pages` defaults to), trimmed to the cap.
pub fn search_pages<S: AppHost>(state: &S, query: String) -> Result<Vec<PageMeta>, String> {
    with_ws(state, |ws| {
        let q = query.trim().to_lowercase();
        let pages = list_pages(ws);
        if q.is_empty() {
            return Ok(pages.into_iter().take(25).collect());
        }
        let mut scored: Vec<(u8, PageMeta)> = pages
            .into_iter()
            .filter_map(|p| {
                let title = p.title.to_lowercase();
                let slug = p.slug.to_lowercase();
                let score = if title == q || slug == q {
                    0
                } else if title.starts_with(&q) || slug.starts_with(&q) {
                    1
                } else if title.contains(&q) || slug.contains(&q) {
                    2
                } else {
                    return None;
                };
                Some((score, p))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
        Ok(scored.into_iter().map(|(_, p)| p).take(25).collect())
    })
}

/// Same shape as [`search_pages`], but filtered to `type:: person`
/// pages and ranked by the shared `outl_actions::search_persons`
/// helper. Powers the `@` mention autocomplete.
pub fn search_persons<S: AppHost>(state: &S, query: String) -> Result<Vec<PageMeta>, String> {
    with_ws(state, |ws| Ok(action_search_persons(ws, &query)))
}

/// Fuzzy-search block text for the `((…))` block-ref autocomplete.
/// Returns each hit's ref handle + a text snippet + hosting slug; the
/// frontend inserts `((<handle>))`, never the display text (block refs
/// resolve by handle). Mirrors the TUI's `candidates_for_blockref`: an
/// empty query returns the most recently created blocks (id descending),
/// a non-empty query delegates to `WorkspaceIndex::search_block_text`.
///
/// The block index isn't held in `AppState`, so this rebuilds it from
/// disk per call — reading and parsing every `.md` + sidecar, O(workspace).
/// That's the same pattern the CLI / MCP block search uses, but here the
/// caller types into `((…))`, so the frontend debounces `search_blocks`
/// (see `BlockRow.refreshSuggest`) to keep the rebuild off the keystroke
/// hot path. Caching the index in `AppState` (invalidated on reload /
/// commit / peer ops) is the real fix and is tracked as a follow-up.
pub fn search_blocks<S: AppHost>(state: &S, query: String) -> Result<Vec<BlockHit>, String> {
    let root = state.storage_root()?;
    let index = WorkspaceIndex::build(&root);
    Ok(collect_block_hits(
        index.block_index(),
        query.trim(),
        BLOCK_HIT_LIMIT,
    ))
}

/// Popup size shared by the empty-query and matched-query paths.
const BLOCK_HIT_LIMIT: usize = 8;

/// The selection + projection logic behind [`search_blocks`], split out
/// so it can be unit-tested against a `BlockIndex` built in-memory
/// (no on-disk workspace, no `AppHost`). `query` is assumed trimmed.
///
/// Empty query → the most recently created blocks (id descending, a
/// fresh block being the likeliest ref target), matching the TUI's
/// `candidates_for_blockref`. Non-empty → the ranked substring matches
/// from the shared `BlockIndex::search_text`.
fn collect_block_hits(index: &BlockIndex, query: &str, limit: usize) -> Vec<BlockHit> {
    if query.is_empty() {
        let mut entries: Vec<&BlockEntry> = index.iter_blocks().collect();
        entries.sort_by_key(|b| std::cmp::Reverse(b.id));
        entries.into_iter().take(limit).map(block_hit).collect()
    } else {
        index
            .search_text(query, limit)
            .into_iter()
            .map(block_hit)
            .collect()
    }
}

/// Project a [`BlockEntry`] onto the wire [`BlockHit`], truncating the
/// text to a single-line snippet so a long block doesn't bloat the popup.
fn block_hit(b: &BlockEntry) -> BlockHit {
    BlockHit {
        handle: b.ref_handle.clone(),
        text: block_snippet(&b.text),
        source_slug: b.source_slug.clone(),
    }
}

/// Collapse a block's text to one trimmed line, capped at 80 chars with
/// an ellipsis. Matches the TUI's `truncate_for_snippet` shape.
fn block_snippet(text: &str) -> String {
    const MAX: usize = 80;
    let one_line = text.replace('\n', " ");
    let trimmed = one_line.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX - 1).collect();
    out.push('…');
    out
}

/// Search the GitHub gemoji catalog for shortcodes matching `query`.
/// Powers the `:shortcode:` autocomplete in the block editor. Wraps
/// `outl_md::emoji::search` so every client ranks identically — no
/// parallel catalog index lives in a frontend bundle.
///
/// Returns up to `limit` hits. Empty / whitespace-only query
/// short-circuits to an empty vec (matches the helper's contract).
pub fn emoji_search(query: String, limit: usize) -> Result<Vec<outl_md::emoji::EmojiHit>, String> {
    Ok(outl_md::emoji::search(&query, limit))
}

pub fn open_today_journal<S: AppHost>(state: &S) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let id = with_ws_mut(state, |ws| {
        open_today(ws, state.hlc()).map_err(|e| e.to_string())
    })?;
    with_ws(state, |ws| {
        // Refresh the `.md` the view reads when a peer's ops moved the tree
        // past the stale projection (issue #166); a no-op when in sync.
        // A refusal comes back as `md_ahead_of_log` and rides the view to
        // the user — the page still opens.
        let ahead = reproject_stale_md(ws, &root, id, "open_today_journal").ahead_of_log;
        let view = build_page_view(ws, &root, id).map_err(|e| e.to_string())?;
        Ok(view.with_ahead_of_log_check(ahead))
    })
}

pub fn open_journal_for<S: AppHost>(state: &S, slug: String) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let date = parse_date(&slug)?;
    let id = with_ws_mut(state, |ws| {
        open_journal(ws, state.hlc(), date).map_err(|e| e.to_string())
    })?;
    with_ws(state, |ws| {
        let ahead =
            reproject_stale_md(ws, &root, id, &format!("open_journal_for {slug}")).ahead_of_log;
        let view = build_page_view(ws, &root, id).map_err(|e| e.to_string())?;
        Ok(view.with_ahead_of_log_check(ahead))
    })
}

/// The command name says "slug" for backward-compat with the frontend,
/// but the input is whatever the user typed / clicked — try the literal
/// first (programmatic callers and the picker pass a clean slug), then
/// fall through to `open_or_create_by_name`, which slugifies the input
/// for the disk path and keeps the original as the page title. That way
/// clicking a ref to a page that doesn't exist still opens a fresh page
/// in the same round-trip.
///
/// Before building the view, lazily project the page's `.md` + sidecar
/// **only when the `.md` is absent from disk**. Without this, a page
/// synced from a peer exists in the in-memory CRDT tree (so it shows up
/// in search) but its `.md` was never written on this device;
/// `read_page_outline` reads through `outl_md::read_for_rewrite`, where
/// a **missing** file legitimately returns `""` (an unreadable one is an
/// error), `parse("")` produces an empty outline, and the page opens
/// blank (issue #120).
/// The `_if_absent` guard avoids rewriting the `.outl` sidecar on every
/// open: `build_sidecar` stamps `last_synced_at: now()`, so an
/// unconditional projection would create constant sync churn on the
/// hottest nav path even when nothing changed.
pub fn open_page_by_slug<S: AppHost>(state: &S, slug: String) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let existing = with_ws(state, |ws| Ok(find_by_slug(ws, &slug)))?;
    let id = match existing {
        Some(id) => id,
        None => with_ws_mut(state, |ws| {
            open_or_create_by_name(ws, state.hlc(), &slug, PageKind::Page)
                .map_err(|e| e.to_string())
        })?,
    };
    let ahead = with_ws_mut(state, |ws| {
        Ok(reproject_stale_md(ws, &root, id, &format!("open_page_by_slug {slug}")).ahead_of_log)
    })?;
    with_ws(state, |ws| {
        let view = build_page_view(ws, &root, id).map_err(|e| e.to_string())?;
        Ok(view.with_ahead_of_log_check(ahead))
    })
}

/// Compute `slug`'s backlinks lazily, off the page-open path.
///
/// The frontend calls this after the outline paints (see
/// [`crate::state::BacklinksReply`]): `backlinks_for_page` is an
/// `O(blocks-in-workspace)` scan, so keeping it out of `build_page_view`
/// is what makes the journal appear instantly on a large workspace. An
/// unknown slug is a soft error the frontend can ignore (the page may
/// have just been deleted).
pub async fn page_backlinks<S: AppHost>(state: &S, slug: String) -> Result<BacklinksReply, String> {
    let root = state.storage_root()?;
    let workspace = state.workspace_arc();
    let index = state.backlink_index();
    compute_backlinks_offloaded(workspace, index, root, slug).await
}

/// Persist the backlinks-list direction (`[display] backlinks_order`,
/// issue #142) and return `slug`'s backlinks re-sorted under the new
/// order.
///
/// A pure display preference — it lives in `config.toml`, never the op
/// log, and does not converge between devices (same policy as the
/// theme). Returns only the re-sorted [`BacklinksReply`] (not a whole
/// `PageView`): the panel already has the outline, it just swaps the
/// backlinks list. Unknown `order` values fall back to `newest` rather
/// than erroring — a UI toggle can't produce anything else.
pub async fn set_backlinks_order<S: AppHost>(
    state: &S,
    order: String,
    slug: String,
) -> Result<BacklinksReply, String> {
    let mut cfg = outl_config::load();
    cfg.display.backlinks_order = match order.as_str() {
        "oldest" => outl_config::BacklinksOrder::Oldest,
        _ => outl_config::BacklinksOrder::Newest,
    };
    outl_config::save(&cfg).map_err(|e| e.to_string())?;

    let root = state.storage_root()?;
    let workspace = state.workspace_arc();
    let index = state.backlink_index();
    compute_backlinks_offloaded(workspace, index, root, slug).await
}

/// Resolve a batch of block ids to the distinct page/journal slugs they
/// belong to — the sync-progress feed's "page X synced" labels.
///
/// The frontend fires this after a `sync-progress` event whose `phase` is
/// `received-ops` carries a (small, capped) `nodes` list; the engine only
/// ships block ids, so the slug resolution happens here where the
/// materialized workspace is. Best-effort: an id not in the tree yet (the
/// reload may still be in flight) or under no registered page root is
/// skipped, so the caller renders whatever resolved. Order-preserving and
/// deduped, so the panel shows the first distinct pages the sync touched.
/// Cheap by construction (the engine caps `nodes`), so it runs inline
/// under the workspace lock rather than off-thread.
pub fn resolve_page_labels<S: AppHost>(
    state: &S,
    node_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    with_ws(state, |ws| {
        let mut seen = std::collections::HashSet::new();
        let mut slugs = Vec::new();
        for id in &node_ids {
            let Ok(node) = parse_node_id(id) else {
                continue;
            };
            if let Some(slug) = ws.slug_for_node(node) {
                if seen.insert(slug.clone()) {
                    slugs.push(slug);
                }
            }
        }
        Ok(slugs)
    })
}

/// Open whatever a user-typed ref / tag / picker entry points at, in one
/// round-trip. `open_or_create_by_ref` is the single decision tree
/// (date → journal, else literal/slugified/title match → existing page,
/// else create), so the command never bubbles `invalid …` back for
/// normal user input and the frontend has no branching to drift.
///
/// Two-phase: resolve-or-create the target NodeId (mutation), then
/// project the page's `.md` + sidecar to disk before building the view.
/// Without the projection, `open_or_create_by_ref` would `Op::Create` +
/// `SetProp` the page into the op log but `pages/<slug>.md` would never
/// land on disk — `WorkspaceIndex` (which parses `.md` from disk) would
/// silently disagree with the tree CRDT, and a peer pulling the
/// workspace would never see the page at all.
pub fn open_ref<S: AppHost>(
    state: &S,
    app: &tauri::AppHandle,
    target: String,
) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let id = with_ws_mut(state, |ws| {
        open_or_create_by_ref(ws, state.hlc(), &target).map_err(|e| e.to_string())
    })?;
    let outcome = with_ws_mut(state, |ws| {
        Ok(reproject_stale_md(
            ws,
            &root,
            id,
            &format!("open_ref {target}"),
        ))
    })?;
    if let Some(msg) = &outcome.other_error {
        // Non-fatal and *transient*: the op log already has the
        // mutation; the `.md` projection will be retried on the next
        // save / by the orphan scanner on the next boot. Still worth a
        // toast — the ref is already in the user's buffer at this point,
        // and silently leaving the `.md` un-projected means the next
        // reopen shows a "link to nothing".
        //
        // `ahead_of_log` deliberately does NOT come through here: it is
        // neither transient nor about this ref, and it rides the
        // `PageView` to a banner that can explain the recovery. A toast
        // that disappears in 4s is the wrong surface for a page that has
        // stopped syncing until the user runs a command.
        let _ = tauri::Emitter::emit(
            app,
            "ref-projection-failed",
            serde_json::json!({ "target": target, "error": msg }),
        );
    }
    with_ws(state, |ws| {
        let view = build_page_view(ws, &root, id).map_err(|e| e.to_string())?;
        Ok(view.with_ahead_of_log_check(outcome.ahead_of_log))
    })
}

pub fn previous_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(previous_journal_date(date)))
}

pub fn next_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(next_journal_date(date)))
}

pub fn today_slug() -> String {
    journal_slug(today())
}

pub fn date_title(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_title(date))
}

/// Resolve (never create) what a ref would land on, for autocomplete
/// previews. Three phases: literal slug match, normalised slug match
/// (`[[avelino/outl]]` resolves to the page stored as `avelino-outl` —
/// the same rule `open_or_create_by_name` uses on creation), and a
/// last-resort case-insensitive title lookup.
pub fn resolve_ref<S: AppHost>(state: &S, target: String) -> Result<Option<PageMeta>, String> {
    with_ws(state, |ws| {
        if let Some(id) = find_by_slug(ws, &target) {
            return Ok(page_meta_action(ws, id));
        }
        let normalised = outl_md::slug::slugify(&target);
        if normalised != target {
            if let Some(id) = find_by_slug(ws, &normalised) {
                return Ok(page_meta_action(ws, id));
            }
        }
        let lower = target.to_lowercase();
        Ok(list_pages(ws)
            .into_iter()
            .find(|p| p.title.to_lowercase() == lower))
    })
}

/// Delete the page identified by `slug` and return a fresh
/// [`PageView`] of today's journal so the caller navigates to a sane
/// page after the deletion (the deleted page no longer exists).
///
/// Two-phase: the CRDT delete (`outl_actions::page::delete` →
/// `Op::Move(node, TRASH_ROOT)`) plus on-disk projection removal
/// (`remove_page_projection`), then announce the op to peers.
///
/// A missing slug surfaces as a string error; the frontend shows a
/// toast instead of silently navigating away. Confirm before calling
/// — this command does not re-prompt.
pub fn delete_page<S: AppHost>(state: &S, slug: String) -> Result<PageView, String> {
    let root = state.storage_root()?;
    // Delete + open-today's-journal in one lock acquire: both are
    // workspace mutations, and keeping them together avoids a second
    // lock round-trip between the delete and the navigation target.
    let (meta, today_id) = with_ws_mut(state, |ws| {
        let meta = outl_actions::delete_page(ws, state.hlc(), &slug).map_err(|e| e.to_string())?;
        let today_id = open_today(ws, state.hlc()).map_err(|e| e.to_string())?;
        Ok((meta, today_id))
    })?;

    // Drop the `.md` + `.outl` so the page vanishes from disk-side
    // listings right away. Idempotent — a missing file is OK.
    if let Err(e) = remove_page_projection(&root, &meta) {
        tracing::warn!(
            "delete_page: could not remove projection for {}: {e}",
            meta.slug
        );
    }

    // Announce to peers so the delete propagates over iroh without
    // waiting for the catch-up re-sync. Mirrors `announce_after_commit`
    // but we can't reuse it because the deleted page no longer has a
    // `PageMeta` to read from the workspace.
    if let Some(transport) = state.sync_transport() {
        transport.announce_local_ops(&meta.slug, state.hlc().next());
    }

    with_ws(state, |ws| {
        build_page_view(ws, &root, today_id).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::{block_snippet, collect_block_hits};
    use outl_core::id::NodeId;
    use outl_md::parse::OutlineNode;
    use outl_md::sidecar::{derive_ref_handle, SidecarBlock};
    use outl_md::BlockIndex;
    use std::path::PathBuf;

    /// Build an in-memory `BlockIndex` from `(slug, [block text])` pairs,
    /// mirroring the on-disk `.md` + `.outl` shape the real
    /// `WorkspaceIndex::build` reads — but without touching the disk.
    /// Returns the index plus each block's derived ref handle, in insert
    /// order, so tests can assert on the handle the picker would insert.
    fn index_of(pages: &[(&str, &[&str])]) -> (BlockIndex, Vec<(String, String)>) {
        let mut idx = BlockIndex::default();
        let mut handles = Vec::new();
        for (slug, texts) in pages {
            let path = PathBuf::from(format!("pages/{slug}.md"));
            let mut ast = Vec::new();
            let mut sidecar = Vec::new();
            for (line, text) in texts.iter().enumerate() {
                let id = NodeId::new();
                let handle = derive_ref_handle(id);
                handles.push((handle.clone(), (*text).to_string()));
                sidecar.push(SidecarBlock {
                    // The handle is captured above so the assertions can
                    // reference it; everything else derives from the text.
                    ref_handle: handle,
                    ..SidecarBlock::from_text(id, line + 1, 0, *text)
                });
                ast.push(OutlineNode {
                    text: (*text).to_string(),
                    properties: Vec::new(),
                    children: Vec::new(),
                });
            }
            idx.collect_page_blocks(slug, &path, &ast, &sidecar);
        }
        (idx, handles)
    }

    #[test]
    fn query_returns_matching_block_handle_and_slug() {
        let (idx, _) = index_of(&[
            ("architecture", &["decide storage backend"]),
            ("journal", &["buy milk"]),
        ]);
        let hits = collect_block_hits(&idx, "storage", 8);
        assert_eq!(hits.len(), 1, "only the matching block should surface");
        let hit = &hits[0];
        assert_eq!(hit.text, "decide storage backend");
        assert_eq!(hit.source_slug, "architecture");
        // The inserted handle is the block's ref handle, never the text.
        assert!(hit.handle.starts_with("blk-"), "got {}", hit.handle);
    }

    #[test]
    fn query_is_case_insensitive() {
        let (idx, _) = index_of(&[("p", &["Decide Storage Backend"])]);
        assert_eq!(collect_block_hits(&idx, "STORAGE", 8).len(), 1);
    }

    #[test]
    fn empty_query_lists_newest_blocks_first() {
        // Explicit, strictly-increasing ULID values so "newest" is
        // unambiguous — `NodeId::new()` within one millisecond has a
        // random tail and is NOT monotonic, so we can't lean on call
        // order. Higher id = newer; the empty-query popup sorts
        // descending, so the largest id must come first.
        let mut idx = BlockIndex::default();
        let mut ast = Vec::new();
        let mut sidecar = Vec::new();
        for (n, text) in [(1u128, "oldest"), (2, "middle"), (3, "newest")] {
            let id = NodeId(ulid::Ulid(n));
            sidecar.push(SidecarBlock::from_text(id, n as usize, 0, text));
            ast.push(OutlineNode {
                text: text.to_string(),
                properties: Vec::new(),
                children: Vec::new(),
            });
        }
        idx.collect_page_blocks("p", &PathBuf::from("pages/p.md"), &ast, &sidecar);

        let hits = collect_block_hits(&idx, "", 8);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].text, "newest");
        assert_eq!(hits[1].text, "middle");
        assert_eq!(hits[2].text, "oldest");
        // Top hit inserts the newest block's handle, not its text.
        assert_eq!(hits[0].handle, derive_ref_handle(NodeId(ulid::Ulid(3))));
    }

    #[test]
    fn limit_caps_the_result_set() {
        let texts: Vec<String> = (0..20).map(|i| format!("block {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let (idx, _) = index_of(&[("p", &refs)]);
        assert_eq!(collect_block_hits(&idx, "", 8).len(), 8);
        assert_eq!(collect_block_hits(&idx, "block", 8).len(), 8);
    }

    #[test]
    fn no_match_returns_empty() {
        let (idx, _) = index_of(&[("p", &["hello world"])]);
        assert!(collect_block_hits(&idx, "zzz", 8).is_empty());
    }

    #[test]
    fn snippet_trims_and_keeps_short_text_verbatim() {
        assert_eq!(block_snippet("  hello world  "), "hello world");
    }

    #[test]
    fn snippet_collapses_newlines_to_spaces() {
        assert_eq!(block_snippet("line one\nline two"), "line one line two");
    }

    #[test]
    fn snippet_truncates_long_text_with_ellipsis() {
        let long = "a".repeat(200);
        let out = block_snippet(&long);
        assert_eq!(out.chars().count(), 80);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn snippet_is_utf8_safe_on_multibyte_boundary() {
        // 100 accented chars — truncation must slice on char, not byte.
        let long = "á".repeat(100);
        let out = block_snippet(&long);
        assert_eq!(out.chars().count(), 80);
        assert!(out.ends_with('…'));
    }
}
