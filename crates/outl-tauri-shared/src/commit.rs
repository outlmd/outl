//! What happens around a page mutation on the Tauri side, and how a
//! commit refreshes the host's cached backlink index.
//!
//! The *sequence* is `outl_actions::commit::commit_page` and has been
//! since [#264](https://github.com/outlmd/outl/issues/264); what lives
//! here is this crate's half of it — `TauriCommitHooks` against
//! [`AppHost`], the three `finish_in_page*` entry points every mutating
//! command funnels through, and the two ways a commit can leave the
//! backlink index ([`BacklinkRefresh`]).
//!
//! **Those are one idea, not two.** The refresh policy is steps 4 and 5
//! of that pipeline, and the whole reason [`BacklinkRefresh::PatchPage`]
//! exists is that the two steps run in that order: the patch reads the
//! `.md` the projection writes, so it cannot be taken where the pipeline
//! puts the backlink step. Splitting the policy from the hooks would put
//! that ordering constraint in one module and the code it constrains in
//! another.
//!
//! Split out of `helpers.rs`, which is argument parsing and lock
//! acquisition and had no reason to own a pipeline.

use std::path::Path;

use outl_actions::{
    apply_page_md_with_sidecar_guarded, commit_page, page_meta as page_meta_action, ActionError,
    CommitHooks,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tracing::warn;

use crate::helpers::{build_page_view, build_page_view_from_tree, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

/// Apply a workspace mutation `f` and project the result back to
/// `.md` + sidecar.
///
/// The op log is the source of truth: the `.md` and the sidecar are
/// projections regenerated after the mutation so what the user reads on
/// disk matches the op-log state. We do **not** run `reconcile_md`
/// before `f` — the op log is already up to date with whatever peers
/// delivered, and "catching up" from a lagging on-disk `.md` would risk
/// emitting Delete cascades.
pub fn finish_in_page<S, F>(state: &S, page_id: NodeId, f: F) -> Result<PageView, String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<(), ActionError>,
{
    finish_in_page_with(state, page_id, f).map(|(_, view)| view)
}

/// Variant of [`finish_in_page`] that also returns whatever value the
/// mutation produced (the new `NodeId` for `create_block`, etc.) so the
/// frontend never has to re-discover it from a DFS diff of the outline.
///
/// When the host exposes undo stacks ([`AppHost::history`] returns
/// `Some`), the pre-mutation `.md` render is recorded — but only when
/// the mutation actually changed the page render, so no-op commands
/// don't turn `Cmd+Z` into a visible nothing.
pub fn finish_in_page_with<S, F, T>(
    state: &S,
    page_id: NodeId,
    f: F,
) -> Result<(T, PageView), String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
{
    finish_in_page_with_refresh(state, page_id, BacklinkRefresh::DropAll, f)
}

/// [`finish_in_page_with`], with the backlink-index policy stated.
///
/// Every other commit takes [`BacklinkRefresh::DropAll`], because a
/// mutation whose reach this crate cannot bound may have changed any
/// page's backlinks. A command that knows it dirtied exactly one page
/// takes [`BacklinkRefresh::PatchPage`] instead and keeps the cached
/// index — see that variant for what it costs.
pub fn finish_in_page_with_refresh<S, F, T>(
    state: &S,
    page_id: NodeId,
    refresh: BacklinkRefresh,
    f: F,
) -> Result<(T, PageView), String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
{
    let root = state.storage_root()?;
    with_ws_mut(state, |ws| {
        let mut hooks = TauriCommitHooks {
            state,
            root: &root,
            refresh,
            projection_failure: None,
        };
        let value = commit_page(ws, &mut hooks, page_id, f).map_err(|e| e.to_string())?;
        let projection_failure = hooks.projection_failure;

        let mut view = if state.projection_writer().is_some() {
            // Async-writes default: the hook queued the write, so the
            // `.md` on disk is momentarily behind. Read the reply off the
            // tree, which is the source of truth either way.
            //
            // `PatchPage` took its write inline and could therefore read
            // disk — but the tree is equally correct and cheaper, and a
            // re-parse here would be a page nobody renders: a cross-page
            // reply is discarded unless the source page is the one on
            // screen (invariant 15's second question).
            build_page_view_from_tree(ws, page_id).map_err(|e| e.to_string())?
        } else {
            build_page_view(ws, &root, page_id).map_err(|e| e.to_string())?
        };
        // A projection failure cannot turn an already-durable mutation
        // into an `Err`, so it rides the successful view. Asked outside
        // the branch above because `PatchPage` takes the write inline
        // even on a host with a writer, so that path can produce one too.
        if let Some(e) = projection_failure {
            let failure = crate::state::ProjectionWriteFailed::from_error(page_id, &e);
            view.md_ahead_of_log = failure.md_ahead_of_log.clone();
            view.projection_error = failure.md_ahead_of_log.is_none().then_some(failure.error);
        }
        Ok((value, view))
    })
}

/// How a commit maintains the host's cached backlink index.
///
/// The index is `O(blocks-in-workspace)` to rebuild from disk, so which
/// of these a commit picks decides what the *next* `page_backlinks`
/// costs — root [`CLAUDE.md`][inv] invariant 15.
///
/// [inv]: https://github.com/outlmd/outl/blob/main/CLAUDE.md#critical-invariants-never-violate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacklinkRefresh {
    /// Drop the whole index; the next `page_backlinks` rebuilds it by
    /// reading and parsing every `.md` in the workspace.
    ///
    /// The default, and right for a mutation whose reach this crate
    /// cannot bound: a paste, a move across pages, a template, a plugin.
    DropAll,
    /// Patch this one page's entries from its fresh `.md` and keep the
    /// rest of the index (`O(one page)`).
    ///
    /// Only correct when the mutation touched exactly this page, which
    /// is why it is opt-in per command rather than the default. It also
    /// **forces a synchronous projection**: the patch reads the `.md`
    /// the projection writes, so a queued write would leave the index
    /// holding the text from before the edit.
    PatchPage,
}

/// The Tauri clients' half of [`commit_page`].
///
/// Holds the borrowed host plus the one thing the sequence produces that
/// the caller still needs: a projection failure. That failure arrives
/// *after* the op log already holds the mutation, which is why
/// [`CommitHooks::project`] returns nothing — the commit succeeded, and
/// what is left is reporting.
struct TauriCommitHooks<'a, S: AppHost> {
    state: &'a S,
    root: &'a Path,
    refresh: BacklinkRefresh,
    projection_failure: Option<ActionError>,
}

impl<S: AppHost> CommitHooks for TauriCommitHooks<'_, S> {
    fn project(&mut self, workspace: &Workspace, page: NodeId) {
        if self.refresh == BacklinkRefresh::PatchPage {
            // The index patch reads this page's `.md`, so the write has
            // to have landed — which rules out queueing it. A cross-page
            // commit pays one page's render + write on this thread and
            // saves the next `page_backlinks` an `O(blocks-in-workspace)`
            // rebuild; `flush()` would cost the same write plus whatever
            // else is in the queue.
            match apply_page_md_with_sidecar_guarded(workspace, self.root, page) {
                Ok(_) => reindex_backlink_page(self.state, workspace, page),
                Err(e) => {
                    warn!("page md+sidecar sync failed: {e}");
                    // Nothing to patch from, so fall back to the policy
                    // every other commit uses rather than leaving the
                    // index describing a page it can no longer read.
                    invalidate_backlink_index(self.state);
                    // `PatchPage` is the cross-page shape, so the reply
                    // describes a page the client is not rendering and
                    // an annotation on it reaches nobody. Same reason
                    // `move_block_after` notifies the source page over
                    // the event bridge instead.
                    if let Some(writer) = self.state.projection_writer() {
                        writer.report_failure(page, &e);
                    }
                    self.projection_failure = Some(e);
                }
            }
            return;
        }
        if let Some(writer) = self.state.projection_writer() {
            // The op log already has the truth, so queue the render +
            // SHA-256 + disk write off-thread. The commit never blocks
            // the next keystroke (the outl async-writes default).
            writer.queue(page);
            return;
        }
        // This *is* step 5 of the commit pipeline, not a caller taking it
        // alone: synchronous fallback for a host with no projection worker.
        if let Err(e) = apply_page_md_with_sidecar_guarded(workspace, self.root, page) {
            warn!("page md+sidecar sync failed: {e}");
            self.projection_failure = Some(e);
        }
    }

    fn records_undo(&self) -> bool {
        self.state.history().is_some()
    }

    fn record_undo(&mut self, page: NodeId, before: String) {
        if let Some(history) = self.state.history() {
            history.lock().entry(page).or_default().record(before);
        }
    }

    fn invalidate_backlinks(&mut self) {
        match self.refresh {
            // Dropped under the workspace lock, so it serializes with the
            // rebuild `page_backlinks` does; the next read rebuilds it.
            BacklinkRefresh::DropAll => invalidate_backlink_index(self.state),
            // Deliberately nothing here. This step runs *before* the
            // projection, and the patch reads the `.md` that step writes,
            // so it is taken at the end of `project` instead.
            BacklinkRefresh::PatchPage => {}
        }
    }

    fn announce(&mut self, workspace: &Workspace, page: NodeId) {
        // One owner: `set_block_collapsed` bypasses the commit pipeline
        // and calls the same function directly.
        announce_after_commit(self.state, workspace, page);
    }
}

/// Drop the host's cached backlinks index so the next `page_backlinks`
/// rebuilds it. Call after any change that can alter backlinks — a local
/// mutation (via [`finish_in_page_with`]) or a peer reload. No-op when
/// the host doesn't cache one ([`AppHost::backlink_index`] is `None`).
///
/// The old index is not freed here. It is a map of small `OutlineNode` /
/// `Arc` allocations (about +49 MB on the maintainer's graph once the
/// citing blocks' children are kept, `docs/performance.md`), and this runs
/// with the workspace lock held on the commit path, so freeing it inline is
/// time the window spends not taking keystrokes. See `discard_slot`.
pub fn invalidate_backlink_index<S: AppHost>(state: &S) {
    if let Some(index) = state.backlink_index() {
        discard_slot(&index);
    }
}

/// Empty `slot`, freeing the old value on a throwaway thread instead of
/// under the caller's locks.
///
/// The slot's guard is released before the value is handed off, so the
/// deallocation holds neither the slot nor (when the caller is a commit)
/// anything the caller took first. A value that cannot get a thread is
/// dropped inline: slower, never leaked.
pub(crate) fn discard_slot<T: Send + 'static>(slot: &Mutex<Option<T>>) {
    let Some(old) = slot.lock().take() else {
        return;
    };
    // The closure owns `old`; if `spawn` fails the closure is dropped with
    // the error, which frees it right here.
    let _ = std::thread::Builder::new()
        .name("backlink-index-drop".into())
        .spawn(move || drop(old));
}

/// Re-index one page's backlinks from its `.md` on disk, in place.
///
/// The incremental counterpart of [`invalidate_backlink_index`], and the
/// GUI twin of the TUI's `reindex_backlinks_for_slug`: editing one page
/// only changes that page's referencing blocks, so re-reading its single
/// `.md` is enough.
///
/// **The page's `.md` must already be projected** — this reads disk, so
/// a caller that queued the write instead of taking it patches the index
/// with the text from before the mutation. [`BacklinkRefresh::PatchPage`]
/// is what guarantees the ordering; nothing here can check it.
///
/// A no-op when the host caches no index ([`AppHost::backlink_index`] is
/// `None`), when the slot is empty (nothing built yet — the next
/// `page_backlinks` builds it), or when the page has no meta.
///
/// Lock order is [`invalidate_backlink_index`]'s: the index lock is
/// taken *inside* whatever holds the workspace, never the other way
/// round (`commands/page_backlinks.rs` carries the ABBA deadlock this
/// avoids).
pub fn reindex_backlink_page<S: AppHost>(state: &S, workspace: &Workspace, page: NodeId) {
    let Some(index) = state.backlink_index() else {
        return;
    };
    let Ok(root) = state.storage_root() else {
        return;
    };
    let Some(meta) = page_meta_action(workspace, page) else {
        return;
    };
    let mut guard = index.lock();
    let Some(index) = guard.as_mut() else {
        return;
    };
    index.reindex_page_from_disk(&meta, &root);
}

/// Post-commit hook: tell connected peers this device just produced new
/// ops so a peer pulls **immediately** over iroh gossip, instead of
/// waiting for the catch-up loop's maintenance re-sync.
///
/// Best-effort and non-fatal by design:
/// - no transport wired (file-sync / not yet started) → nothing to
///   announce;
/// - the announce never crosses (flaky link) → the catch-up loop's
///   periodic re-sync still converges.
///
/// This is the GUI mirror of the TUI's `save()` tail. Without it, edits
/// committed the op locally but never woke peers — so propagation
/// depended entirely on the catch-up timing.
///
/// Called through [`CommitHooks::announce`] for every commit, and
/// directly by `commands::block::set_block_collapsed`, which skips the
/// commit pipeline on purpose (it does not reproject).
///
/// The transport check comes **before** `page_meta` deliberately: a host
/// running the file transport has nothing to announce, and resolving the
/// slug costs five property lookups plus a `block_text` on a path that
/// runs once per keystroke commit.
pub fn announce_after_commit<S: AppHost>(state: &S, ws: &Workspace, page_id: NodeId) {
    let Some(transport) = state.sync_transport() else {
        return;
    };
    let Some(meta) = page_meta_action(ws, page_id) else {
        return;
    };
    transport.announce_local_ops(&meta.slug, state.hlc().next());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread::ThreadId;

    /// Reports, on drop, which thread freed it and whether the slot it came
    /// out of was lockable at that moment.
    struct Probe {
        slot: Arc<Mutex<Option<Probe>>>,
        report: mpsc::Sender<(ThreadId, bool)>,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            let slot_free = self.slot.try_lock().is_some();
            let _ = self.report.send((std::thread::current().id(), slot_free));
        }
    }

    #[test]
    fn the_old_value_is_freed_off_the_calling_thread_and_off_the_slot() {
        let (tx, rx) = mpsc::channel();
        let slot = Arc::new(Mutex::new(None));
        *slot.lock() = Some(Probe {
            slot: slot.clone(),
            report: tx,
        });

        discard_slot(&*slot);

        let (freed_on, slot_free) = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the old value was dropped");
        assert_ne!(
            freed_on,
            std::thread::current().id(),
            "freed inline: the caller's locks were held for the whole deallocation"
        );
        assert!(slot_free, "the slot guard was still held while it dropped");
        assert!(slot.lock().is_none(), "the slot is left empty");
    }

    #[test]
    fn an_empty_slot_is_a_no_op() {
        let slot: Mutex<Option<u8>> = Mutex::new(None);
        discard_slot(&slot);
        assert!(slot.lock().is_none());
    }
}
