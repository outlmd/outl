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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    build_backlink_index_from_disk, find_by_slug, list_pages, namespace_descendants,
    page_meta as page_meta_action, sort_backlinks, BacklinkIndex, OutlineNode, PageMeta,
};
use outl_core::workspace::Workspace;
use parking_lot::Mutex;

use crate::helpers::parse_node_id;
use crate::offload;
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
    // The command this module's shape was first proven by, now routed
    // through the shared owner of that shape (`crate::offload`) so there
    // is one `spawn_blocking` call site and one join-error wording. The
    // lock dance below stays hand-rolled on purpose: its retry loop
    // `continue`s out of the locked region, which `offload::locked`
    // cannot express, and every line of it is load-bearing.
    offload::run("page_backlinks", move || {
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
                let looked = {
                    let guard = workspace.lock();
                    let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
                    look_up(&idx, ws, &meta)
                };
                return Ok(shape_reply(looked, &meta, &metas, backlinks_order));
            }
        };
        // Phase 2: build the index FROM DISK when stale — reads the `.md`
        // projection, touches no `Workspace`, holds NO lock. This is what
        // keeps opening the journal / pressing Esc from freezing: the
        // O(blocks) work never materializes the vault and never blocks an
        // edit waiting on the workspace lock.
        //
        // Phase 3: O(refs) lookup under a brief lock (`for_page` reads the
        // page's own `template::` property; no block-text scan). Only the
        // lookup is under it — and that includes fetching each direct
        // hit's subtree, which is a refcount bump (`look_up`), never the
        // deep copy: a synchronous `#[tauri::command]` runs on the
        // IPC thread, so whatever this holds the workspace for is time the
        // window is not taking keystrokes (`UX.md` → Never block a
        // keystroke). Sorting, the shallow projection and the namespace
        // walk need no `Workspace`, so `shape_reply` runs after both locks
        // drop.
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
            let looked = {
                let guard = workspace.lock();
                let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
                let g = index.lock();
                match g.as_ref() {
                    Some(idx) => look_up(idx, ws, &meta),
                    // Invalidated between the build and the lookup: drop
                    // both locks and rebuild.
                    None => continue,
                }
            };
            return Ok(shape_reply(looked, &meta, &metas, backlinks_order));
        }
        // Commits are invalidating faster than the cache refills; serve a
        // one-shot from-disk build directly, same as the host-without-a-slot
        // path above (marginally stale is fine, the next refresh re-caches).
        let idx = build_backlink_index_from_disk(&metas, &root);
        let looked = {
            let guard = workspace.lock();
            let ws = guard.as_ref().ok_or_else(|| ERR_LOADING.to_string())?;
            look_up(&idx, ws, &meta)
        };
        Ok(shape_reply(looked, &meta, &metas, backlinks_order))
    })
    .await
}

/// How many namespace-only backlinks ride the wire per page.
///
/// The set is unbounded — a namespace with thousands of mentions is
/// normal on a real graph — and a client renders it as a collapsed
/// section, so the whole list was never going to be read. 50 is enough
/// to be useful when opened and small enough that the count, not the
/// payload, carries the scale.
pub(crate) const NAMESPACE_BACKLINK_CAP: usize = 50;

/// How many subtree nodes one `page_backlinks` reply may carry, summed
/// over every direct hit.
///
/// Direct hits have no breadth cap of their own, and a citing block's
/// children are four levels deep. The busiest page on the maintainer's
/// graph (853 direct hits, 3,572 subtree nodes behind 615 of them) went
/// from 753 KB to 2,486 KB per reply, about 0.5 KB per node, and the
/// panel refetches on every peer reload and every cross-page commit.
/// 2,000 nodes keeps that page near 1.7 MB (derived from the measured
/// per-node cost, not re-measured) and leaves an ordinary page, whose
/// citing blocks have a handful of children each, entirely unaffected.
/// Numbers in `docs/performance.md`.
pub const SUBTREE_NODE_BUDGET: usize = 2_000;

/// Spends [`SUBTREE_NODE_BUDGET`] in display order, **as a prefix**: the
/// first hit whose subtree does not fit closes the budget for it and
/// every hit after, so what a reader sees is "rows from the top have
/// their children, rows below do not" and never a gap in the middle.
struct SubtreeBudget {
    left: usize,
    open: bool,
}

impl SubtreeBudget {
    fn new(nodes: usize) -> Self {
        Self {
            left: nodes,
            open: true,
        }
    }

    /// Take `nodes` out of the budget, or close it. Counting is a read
    /// of an `Arc` the index already holds: nothing is allocated for a
    /// hit that is turned away.
    fn admit(&mut self, nodes: usize) -> bool {
        if self.open && nodes <= self.left {
            self.left -= nodes;
            true
        } else {
            self.open = false;
            false
        }
    }
}

/// Nodes in a forest, the node itself included.
fn node_count(nodes: &[OutlineNode]) -> usize {
    nodes.iter().map(|n| 1 + node_count(&n.children)).sum()
}

/// A lookup's answer: the split, plus each **direct** hit's stored
/// subtree, in the same order. Everything in it is a leaf clone or a
/// refcount, so it is safe to build while both locks are held.
struct Looked {
    split: outl_actions::SplitBacklinks,
    /// Parallel to `split.direct`. The deep copy into the reply is
    /// [`shape_reply`]'s, after the locks drop.
    subtrees: Vec<Option<Arc<[OutlineNode]>>>,
    /// Ids of the shipped nodes the op log has folded. See [`collapsed_ids`].
    collapsed: HashSet<String>,
}

/// The phase-3 lookup. `for_page_split` returns shallow leaves, and
/// `BacklinkIndex::subtree` is one hash lookup + `Arc` clone per direct
/// hit — together `O(refs)` with no subtree copied under the lock.
fn look_up(idx: &BacklinkIndex, ws: &Workspace, meta: &PageMeta) -> Looked {
    let split = idx.for_page_split(ws, meta);
    let subtrees: Vec<_> = split
        .direct
        .iter()
        .map(|b| idx.subtree(&b.block_id))
        .collect();
    let collapsed = collapsed_ids(ws, &split.direct, &subtrees);
    Looked {
        split,
        subtrees,
        collapsed,
    }
}

/// Which of the direct hits and their stored subtrees are folded.
///
/// **The index cannot answer this, and must not try.** It is built from
/// the `.md`, and `Op::SetCollapsed` lives only in the op log: the fold
/// was moved out of the sidecar because a per-file write lost flips under
/// iCloud's last-write-wins (root `CLAUDE.md` invariant 7). So every
/// `collapsed` the index holds is `false`, and a fold made from a
/// backlink row never came back on the refetch. The page's own outline
/// has the same split and the same answer
/// (`read_page_outline_with_workspace` overlays the tree).
///
/// Read under the lookup's lock because the tree is the only place the
/// flag is. The cost is one `HashSet` probe per shipped node, bounded by
/// what the lookup already returns, and no node is copied here.
fn collapsed_ids(
    ws: &Workspace,
    direct: &[outl_actions::Backlink],
    subtrees: &[Option<Arc<[OutlineNode]>>],
) -> HashSet<String> {
    fn visit(ws: &Workspace, nodes: &[OutlineNode], out: &mut HashSet<String>) {
        for node in nodes {
            note_if_folded(ws, &node.id, out);
            visit(ws, &node.children, out);
        }
    }
    let mut out = HashSet::new();
    for b in direct {
        note_if_folded(ws, &b.block_id, &mut out);
    }
    for kids in subtrees.iter().flatten() {
        visit(ws, kids, &mut out);
    }
    out
}

/// A transient id (a block the sidecar does not cover) parses, but the
/// tree has never seen it, so it reads as expanded, the outline's rule.
fn note_if_folded(ws: &Workspace, id: &str, out: &mut HashSet<String>) {
    if parse_node_id(id).is_ok_and(|n| ws.tree().is_collapsed(n)) {
        out.insert(id.to_string());
    }
}

/// Write the folded flags back onto a node and its shipped subtree.
fn mark_collapsed(node: &mut OutlineNode, collapsed: &HashSet<String>) {
    node.collapsed = collapsed.contains(&node.id);
    for child in &mut node.children {
        mark_collapsed(child, collapsed);
    }
}

/// Shape a [`Looked`] into the reply, **holding no lock**.
///
/// A direct hit ships with its children (the capped subtree, copied here,
/// once per hit, **after** the sort, so [`SUBTREE_NODE_BUDGET`] is spent
/// on the rows the user sees first; hits past it ship shallow). The namespace
/// set ships through `into_shallow`: its rows are capped, collapsed and
/// text-only, and it can be drawn from thousands of hits.
///
/// Takes the lookup rather than the index + `&Workspace` on purpose: the
/// lookup is the only part that needs the workspace, and every sync
/// `#[tauri::command]` runs on the IPC thread, so time spent holding that
/// lock is time the window spends not accepting keystrokes. Sorting, the
/// `O(hits)` subtree copy and the `O(pages)` namespace walk all run after
/// the caller's locks drop.
fn shape_reply(
    looked: Looked,
    meta: &PageMeta,
    metas: &[PageMeta],
    backlinks_order: outl_config::BacklinksOrder,
) -> BacklinksReply {
    let Looked {
        split,
        subtrees,
        collapsed,
    } = looked;
    // Keyed by block id because the sort below moves the hits: the budget
    // is spent in the order the user sees, not the order the index walked.
    let mut subtrees: HashMap<String, Arc<[OutlineNode]>> = split
        .direct
        .iter()
        .zip(subtrees)
        .filter_map(|(b, kids)| Some((b.block_id.clone(), kids?)))
        .collect();
    let mut backlinks = split.direct;
    sort_backlinks(&mut backlinks, backlinks_order.newest_first());
    let mut budget = SubtreeBudget::new(SUBTREE_NODE_BUDGET);
    let backlinks: Vec<_> = backlinks
        .into_iter()
        .map(|b| match subtrees.remove(&b.block_id) {
            Some(kids) if budget.admit(node_count(&kids)) => b.with_children(&kids),
            _ => b,
        })
        .map(|mut b| {
            if !collapsed.is_empty() {
                mark_collapsed(&mut b.source_block, &collapsed);
            }
            b
        })
        .collect();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_is_a_prefix_not_a_best_fit() {
        let mut b = SubtreeBudget::new(10);
        assert!(b.admit(4));
        assert!(b.admit(6), "exactly the remainder fits");
        assert!(!b.admit(1), "spent");
        let mut b = SubtreeBudget::new(10);
        assert!(b.admit(4));
        assert!(!b.admit(7), "does not fit");
        assert!(!b.admit(1), "and a smaller one behind it is not let past");
    }
}
