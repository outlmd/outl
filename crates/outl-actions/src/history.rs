//! Bounded undo / redo snapshot stacks shared by GUI clients.
//!
//! Two bounded stacks with vim semantics: every committed mutation
//! pushes a snapshot onto `undo`; `undo` pops to `redo`; `redo` pops
//! back to `undo`; a **new edit clears `redo`** (a new edit branches
//! history).
//!
//! The engine is generic over the snapshot type because each client
//! decides what a "page state" is on its surface:
//!
//! - the desktop snapshots the page's rendered `.md`
//!   (`journal::render_page_md`) per mutation and restores it with
//!   `restore_page_md` below;
//! - a future mobile undo can reuse the same pair unchanged.
//!
//! Restores route through `outl_md::reconcile_md` — the snapshot is
//! written to the page's `.md` and reconciled back (matching → diff →
//! ops through `Workspace::apply`), so an undo is *new ops in the
//! log*, never a rewrite of it. The op log stays the single source of
//! truth (invariant #1).
//!
//! This is **not** an in-flight text-editing undo: per-keystroke
//! history inside an uncommitted draft belongs to the client's editor
//! widget (the TUI's `EditBuffer`, the browser textarea), not here.

use std::path::Path;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::journal::{page_md_path, write_md_atomic};
use crate::page::page_meta;

/// Default bound for each stack. Matches the TUI's session history
/// cap so a long session can't blow memory.
pub const DEFAULT_HISTORY_CAP: usize = 200;

/// Bounded undo / redo stacks over an arbitrary snapshot type.
#[derive(Debug, Clone)]
pub struct HistoryStacks<T> {
    undo: Vec<T>,
    redo: Vec<T>,
    cap: usize,
}

impl<T> Default for HistoryStacks<T> {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_CAP)
    }
}

impl<T> HistoryStacks<T> {
    /// Empty stacks holding at most `cap` snapshots per side.
    pub fn new(cap: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap,
        }
    }

    /// Record the pre-mutation state. Clears the redo stack — vim
    /// semantics: a new edit branches history.
    pub fn record(&mut self, snapshot: T) {
        self.undo.push(snapshot);
        if self.undo.len() > self.cap {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Pop the most recent snapshot, stashing `current` on the redo
    /// stack. Returns `None` (and leaves `current` unstashed) when
    /// there is nothing to undo.
    pub fn undo(&mut self, current: T) -> Option<T> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        if self.redo.len() > self.cap {
            self.redo.remove(0);
        }
        Some(prev)
    }

    /// Inverse of `undo`: pop from redo, stashing `current` back on
    /// the undo stack.
    pub fn redo(&mut self, current: T) -> Option<T> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        if self.undo.len() > self.cap {
            self.undo.remove(0);
        }
        Some(next)
    }

    /// Attempt an undo, committing the stack transition only when `apply`
    /// succeeds. On failure the selected snapshot is restored in place.
    pub fn try_undo<E>(
        &mut self,
        current: T,
        apply: impl FnOnce(&T) -> Result<(), E>,
    ) -> Option<Result<(), E>> {
        let snapshot = self.undo.pop()?;
        if let Err(error) = apply(&snapshot) {
            self.undo.push(snapshot);
            return Some(Err(error));
        }
        self.redo.push(current);
        if self.redo.len() > self.cap {
            self.redo.remove(0);
        }
        Some(Ok(()))
    }

    /// Attempt a redo, committing the stack transition only when `apply`
    /// succeeds. On failure the selected snapshot is restored in place.
    pub fn try_redo<E>(
        &mut self,
        current: T,
        apply: impl FnOnce(&T) -> Result<(), E>,
    ) -> Option<Result<(), E>> {
        let snapshot = self.redo.pop()?;
        if let Err(error) = apply(&snapshot) {
            self.redo.push(snapshot);
            return Some(Err(error));
        }
        self.undo.push(current);
        if self.undo.len() > self.cap {
            self.undo.remove(0);
        }
        Some(Ok(()))
    }

    /// Whether an `undo` would succeed.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a `redo` would succeed.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Drop both stacks. Call when the snapshots no longer describe
    /// the live workspace — e.g. on workspace switch, or after a peer
    /// merge **changed this page** (restoring a pre-merge snapshot
    /// would silently revert the peer's edits). Don't clear for peer
    /// merges that left the page untouched; the stacks are still
    /// valid and the user keeps their undo depth.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Restore a page to a previously-captured `.md` snapshot.
///
/// Writes `md` to the page's canonical path and reconciles it back
/// into the workspace — the 3-level matcher diffs the snapshot
/// against the current sidecar and emits the minimum op sequence
/// through `Workspace::apply`, so the restore converges across peers
/// like any other edit and the op log is never rewritten.
pub fn restore_page_md(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    root: &Path,
    page_id: NodeId,
    md: &str,
) -> Result<(), ActionError> {
    let meta = page_meta(ws, page_id).ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    let path = page_md_path(root, &meta);
    let _lock = crate::journal::apply::ProjectionLock::acquire(&path)?;
    let current_disk = std::fs::read_to_string(&path)?;
    let expected_current = crate::journal::render_page_md(ws, page_id);
    if current_disk != expected_current {
        return Err(ActionError::PageMarkdownChangedDuringProjection(
            path.display().to_string(),
        ));
    }
    write_md_atomic(&path, md)?;
    // An undo restores an *older* `.md`, so blocks created after the
    // snapshot legitimately fall to matching level 3 and get trashed.
    // That is precisely the case worth recording: it's the one path
    // where a user action deletes blocks wholesale, and `orphans.log`
    // is how they get them back if the undo went further than intended.
    let orphans = crate::sync::orphans_log_path(root);
    if let Err(error) = outl_md::reconcile::reconcile_md(ws, hlc, &path, Some(&orphans)) {
        // BulkDelete is the preflight refusal guaranteed to leave the tree
        // untouched. Put the bytes we replaced back while the page lock is
        // still held, but only if nobody changed the snapshot meanwhile.
        // Other errors may happen after ops were applied; rolling back only
        // their file would make disk disagree with the now-advanced tree.
        if matches!(&error, outl_md::ReconcileError::BulkDelete(_))
            && std::fs::read_to_string(&path).is_ok_and(|current| current == md)
        {
            write_md_atomic(&path, &current_disk)?;
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{append_block, edit_text};
    use crate::journal::{apply_page_md_with_sidecar, render_page_md};
    use crate::page::{open_or_create, PageKind};
    use outl_core::id::ActorId;
    use tempfile::TempDir;

    #[test]
    fn undo_redo_round_trip() {
        let mut h: HistoryStacks<&str> = HistoryStacks::new(10);
        assert!(!h.can_undo());
        assert!(h.undo("now").is_none());

        h.record("v1");
        h.record("v2");
        assert_eq!(h.undo("v3"), Some("v2"));
        assert!(h.can_redo());
        assert_eq!(h.redo("v2"), Some("v3"));
        assert_eq!(h.undo("v3"), Some("v2"));
        assert_eq!(h.undo("v2"), Some("v1"));
        assert!(h.undo("v1").is_none());
    }

    #[test]
    fn record_clears_redo() {
        let mut h: HistoryStacks<i32> = HistoryStacks::new(10);
        h.record(1);
        assert_eq!(h.undo(2), Some(1));
        assert!(h.can_redo());
        h.record(1);
        assert!(!h.can_redo(), "a new edit must branch history");
    }

    #[test]
    fn cap_evicts_oldest() {
        let mut h: HistoryStacks<i32> = HistoryStacks::new(2);
        h.record(1);
        h.record(2);
        h.record(3);
        assert_eq!(h.undo(4), Some(3));
        assert_eq!(h.undo(3), Some(2));
        assert!(h.undo(2).is_none(), "snapshot 1 must have been evicted");
    }

    /// Replicates the desktop's exact record / undo loop
    /// (`finish_in_page_with` + `step_history`) across **multiple**
    /// consecutive undos — pins that the engine walks the whole
    /// stack, not just the most recent mutation.
    #[test]
    fn consecutive_undos_walk_back_multiple_mutations() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let tmp = TempDir::new().unwrap();

        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        let block = append_block(&mut ws, &hlc, Some(page), Some("one")).unwrap();
        apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

        let mut h: HistoryStacks<String> = HistoryStacks::default();

        // Three committed mutations, each recorded the way
        // `finish_in_page_with` does (pre-mutation render).
        for text in ["two", "three", "four"] {
            let before = render_page_md(&ws, page);
            edit_text(&mut ws, &hlc, block, text).unwrap();
            assert_ne!(render_page_md(&ws, page), before);
            h.record(before);
            apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
        }
        // In-app pages carry their title as a `title::` page property, so
        // every render leads with it (see `page::open_or_create`).
        assert_eq!(render_page_md(&ws, page), "title:: Ideas\n\n- four\n");

        // Three undos, each the way `step_history` does it.
        for expected in [
            "title:: Ideas\n\n- three\n",
            "title:: Ideas\n\n- two\n",
            "title:: Ideas\n\n- one\n",
        ] {
            let current = render_page_md(&ws, page);
            let snapshot = h.undo(current).expect("stack must not run dry");
            restore_page_md(&mut ws, &hlc, tmp.path(), page, &snapshot).unwrap();
            assert_eq!(render_page_md(&ws, page), expected);
        }
        let current = render_page_md(&ws, page);
        assert!(h.undo(current).is_none(), "exactly three steps recorded");

        // And redo walks forward again.
        for expected in [
            "title:: Ideas\n\n- two\n",
            "title:: Ideas\n\n- three\n",
            "title:: Ideas\n\n- four\n",
        ] {
            let current = render_page_md(&ws, page);
            let snapshot = h.redo(current).expect("redo stack must not run dry");
            restore_page_md(&mut ws, &hlc, tmp.path(), page, &snapshot).unwrap();
            assert_eq!(render_page_md(&ws, page), expected);
        }
    }

    #[test]
    fn restore_page_md_round_trips_through_ops() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let tmp = TempDir::new().unwrap();

        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        let block = append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
        apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
        let snapshot = render_page_md(&ws, page);
        let ops_before = ws.log().len();

        edit_text(&mut ws, &hlc, block, "changed").unwrap();
        apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
        assert_eq!(ws.block_text(block).as_deref(), Some("changed"));

        restore_page_md(&mut ws, &hlc, tmp.path(), page, &snapshot).unwrap();

        // Note: a >20% text rewrite drops to level-3 matching, so the
        // restored block may carry a fresh NodeId — same semantics as
        // the TUI's snapshot undo, which restores through the same
        // reconcile path. The projection is what must round-trip.
        assert_eq!(
            render_page_md(&ws, page),
            snapshot,
            "undo must restore the page projection"
        );
        assert!(
            ws.log().len() > ops_before,
            "the restore must append ops, never rewind the log"
        );
        let on_disk = std::fs::read_to_string(tmp.path().join("pages/ideas.md")).unwrap();
        assert_eq!(on_disk, snapshot, "the .md on disk must match the snapshot");
    }
}
