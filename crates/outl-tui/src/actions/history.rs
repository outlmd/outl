//! Undo / redo via [`HistorySnapshot`] stacks.
//!
//! Two bounded stacks. Each structural mutation pushes a snapshot
//! onto `undo`; `undo` pops to `redo`; `redo` pops back to `undo`.
//! A new edit clears `redo` — vim semantics (a new edit branches
//! history).

use crate::outline_ops::flat_count;
use crate::state::{App, HistorySnapshot, MAX_HISTORY};
use std::path::Path;

impl App {
    /// Push the current page state onto the undo stack. Clears the redo
    /// stack — that's vim semantics: a new edit branches history.
    pub(crate) fn snapshot_for_undo(&mut self) {
        let snap = HistorySnapshot {
            page: self.page.clone(),
            selected: self.selected,
            cursor_col: self.cursor_col,
            view_path: self.current_path(),
        };
        self.undo.push(snap);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Can the snapshot on top of `stack` be applied to the page on
    /// screen?
    ///
    /// Asked **before** either stack is touched. A snapshot belonging
    /// to another view cannot be restored, and popping first meant the
    /// refusal destroyed it: `restore_snapshot` takes the snapshot by
    /// value, so the entry was gone and `redo` had gained a snapshot of
    /// a page nothing had undone ([#430](https://github.com/outlmd/outl/issues/430)).
    /// Nothing clears these stacks on navigation, so a few presses on
    /// the wrong page emptied the user's history in silence.
    fn top_applies_here(stack: &[HistorySnapshot], here: &Path) -> bool {
        stack.last().is_some_and(|s| s.view_path == here)
    }

    /// Pop the most recent snapshot off the undo stack and restore it.
    pub(crate) fn undo(&mut self) {
        let here = self.current_path();
        if self.undo.is_empty() {
            self.status = "nothing to undo".into();
            return;
        }
        if !Self::top_applies_here(&self.undo, &here) {
            self.status = "no undo for this page".into();
            return;
        }
        let prev = self.undo.pop().expect("checked non-empty above");
        // Capture current state on redo before swapping back.
        let current = HistorySnapshot {
            page: self.page.clone(),
            selected: self.selected,
            cursor_col: self.cursor_col,
            view_path: here,
        };
        self.redo.push(current);
        if self.redo.len() > MAX_HISTORY {
            self.redo.remove(0);
        }
        self.restore_snapshot(prev);
        self.status = "undid".into();
    }

    /// Pop from redo and apply.
    pub(crate) fn redo(&mut self) {
        let here = self.current_path();
        if self.redo.is_empty() {
            self.status = "nothing to redo".into();
            return;
        }
        if !Self::top_applies_here(&self.redo, &here) {
            self.status = "no redo for this page".into();
            return;
        }
        let next = self.redo.pop().expect("checked non-empty above");
        let current = HistorySnapshot {
            page: self.page.clone(),
            selected: self.selected,
            cursor_col: self.cursor_col,
            view_path: here,
        };
        self.undo.push(current);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.restore_snapshot(next);
        self.status = "redid".into();
    }

    /// Replace the in-memory page with a previous snapshot and persist
    /// the change. If the snapshot belongs to a different view (the
    /// user navigated away after the edit), the snapshot is dropped and
    /// the user is notified.
    fn restore_snapshot(&mut self, snap: HistorySnapshot) {
        if snap.view_path != self.current_path() {
            self.status = "skipped: snapshot belongs to a different page".into();
            return;
        }
        self.page = snap.page;
        self.flat_len = flat_count(&self.page.blocks);
        self.selected = snap.selected.min(self.flat_len.saturating_sub(1));
        self.cursor_col = snap.cursor_col.min(self.current_block_char_count());
        // Persist (re-renders .md and reconciles).
        self.save();
    }
}

#[cfg(test)]
mod tests {
    //! The stacks must survive a press on the wrong page.
    //!
    //! `undo` and `redo` both popped before asking whether the
    //! snapshot could be applied, and `restore_snapshot` refuses one
    //! belonging to another view. The popped value was taken by value,
    //! so the refusal destroyed it (issue #430): a few presses after
    //! navigating away silently emptied the user's undo history.

    use crate::state::{App, View};
    use outl_core::{ActorId, Workspace};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        app.page.blocks.clear();
        app.page.blocks.push(outl_md::parse::OutlineNode {
            text: "ship it".to_string(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
        (app, dir)
    }

    /// A view that is definitely not the one `fresh_app` starts on.
    fn elsewhere(dir: &TempDir) -> View {
        View::Page(dir.path().join("pages").join("elsewhere.md"))
    }

    #[test]
    fn undo_on_another_page_keeps_the_snapshot() {
        let (mut app, dir) = fresh_app();
        app.snapshot_for_undo();
        let before: Vec<PathBuf> = app.undo.iter().map(|s| s.view_path.clone()).collect();

        app.view = elsewhere(&dir);
        app.undo();

        let after: Vec<PathBuf> = app.undo.iter().map(|s| s.view_path.clone()).collect();
        assert_eq!(
            after, before,
            "the undo stack lost an entry to a press on the wrong page"
        );
    }

    #[test]
    fn undo_on_another_page_does_not_push_a_bogus_redo() {
        let (mut app, dir) = fresh_app();
        app.snapshot_for_undo();

        app.view = elsewhere(&dir);
        app.undo();

        assert!(
            app.redo.is_empty(),
            "redo took a snapshot of a page that was never undone"
        );
    }

    #[test]
    fn repeated_undo_on_another_page_never_drains_the_stack() {
        let (mut app, dir) = fresh_app();
        app.snapshot_for_undo();
        app.snapshot_for_undo();
        app.snapshot_for_undo();

        app.view = elsewhere(&dir);
        for _ in 0..5 {
            app.undo();
        }

        assert_eq!(
            app.undo.len(),
            3,
            "five presses on the wrong page ate the history"
        );
    }

    #[test]
    fn redo_on_another_page_keeps_the_snapshot() {
        let (mut app, dir) = fresh_app();
        // Get one entry onto `redo` the honest way: snapshot, then undo
        // on the page the snapshot belongs to.
        app.snapshot_for_undo();
        app.undo();
        assert_eq!(app.redo.len(), 1, "setup: expected one redo entry");

        app.view = elsewhere(&dir);
        app.redo();

        assert_eq!(
            app.redo.len(),
            1,
            "the redo stack lost an entry to a press on the wrong page"
        );
    }

    #[test]
    fn undo_on_its_own_page_still_consumes_the_snapshot() {
        let (mut app, _d) = fresh_app();
        app.snapshot_for_undo();
        app.undo();
        assert!(
            app.undo.is_empty(),
            "the guard must not block an undo that does apply"
        );
        assert_eq!(app.redo.len(), 1);
    }
}
