//! Visual mode: a Normal-mode sibling that adds a range anchor so the
//! user can act on N blocks at once (delete, indent, outdent).

use crate::outline_ops::{
    delete_at_path, flat_count, indent_at_path, move_down_at_path, move_up_at_path, node_at_path,
    outdent_at_path, path_for_index,
};
use crate::state::{App, Mode};

impl App {
    /// Enter Visual mode anchored at the current selection.
    pub(crate) fn enter_visual(&mut self) {
        self.mode = Mode::Visual {
            anchor: self.selected,
        };
    }

    /// `gv` — re-enter Visual mode at the last range. No-op when no
    /// Visual session has ever happened in this app instance (the
    /// status line surfaces "no previous selection").
    pub(crate) fn reselect_last_visual(&mut self) {
        let Some((lo, hi)) = self.last_visual else {
            self.status = "no previous selection".into();
            return;
        };
        let max_idx = self.flat_len.saturating_sub(1);
        let lo = lo.min(max_idx);
        let hi = hi.min(max_idx);
        // Re-anchor at `lo` and move selection to `hi` so the range
        // re-renders identically — `visual_range()` swaps them on
        // demand so direction doesn't matter.
        self.mode = Mode::Visual { anchor: lo };
        self.selected = hi;
    }

    /// Snapshot the current Visual range into `last_visual` so a
    /// future `gv` can restore it. Call this right before leaving
    /// Visual mode — every exit path (Esc, yank, delete, indent,
    /// outdent) goes through here.
    pub(crate) fn remember_visual_range(&mut self) {
        if let Some(range) = self.visual_range() {
            self.last_visual = Some(range);
        }
    }

    /// `Esc` from Visual — capture the range, drop back to Normal.
    /// The renderer already shows Normal-mode chrome once `Mode`
    /// changes; nothing else to do.
    pub(crate) fn exit_visual(&mut self) {
        self.remember_visual_range();
        self.mode = Mode::Normal;
    }

    /// Return the (lo, hi) flat indices of the Visual selection,
    /// inclusive on both sides. `None` if not in Visual mode.
    pub(crate) fn visual_range(&self) -> Option<(usize, usize)> {
        if let Mode::Visual { anchor } = self.mode {
            let lo = anchor.min(self.selected);
            let hi = anchor.max(self.selected);
            Some((lo, hi))
        } else {
            None
        }
    }

    /// Delete every block in the Visual range, exit Visual mode.
    pub(crate) fn delete_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        self.snapshot_for_undo();
        self.remember_visual_range();
        // Delete from hi down to lo so flat indices don't shift mid-loop.
        for idx in (lo..=hi).rev() {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                delete_at_path(&mut self.page.blocks, &path);
            }
        }
        self.mode = Mode::Normal;
        self.selected = lo.min(flat_count(&self.page.blocks).saturating_sub(1));
        self.save();
    }

    /// Indent every block in the Visual range. Skips blocks that can't
    /// indent (no previous sibling); the range as a whole still moves
    /// as best it can.
    pub(crate) fn indent_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        self.snapshot_for_undo();
        // Indent in increasing order — earlier blocks moving deeper
        // doesn't change later blocks' flat indices (the count is
        // preserved). Indents that fail (first child of parent) are
        // silently skipped.
        for idx in lo..=hi {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                let _ = indent_at_path(&mut self.page.blocks, &path);
            }
        }
        self.save();
    }

    /// Outdent every block in the Visual range.
    pub(crate) fn outdent_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        self.snapshot_for_undo();
        for idx in lo..=hi {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                let _ = outdent_at_path(&mut self.page.blocks, &path);
            }
        }
        self.save();
    }

    /// Does the block at `path` have a sibling after it? Used to detect
    /// a blocked range reorder before paying for a snapshot + save.
    fn has_next_sibling(&self, path: &[usize]) -> bool {
        let Some((&last, parent)) = path.split_last() else {
            return false;
        };
        let siblings = if parent.is_empty() {
            &self.page.blocks
        } else {
            match node_at_path(&self.page.blocks, parent) {
                Some(node) => &node.children,
                None => return false,
            }
        };
        last + 1 < siblings.len()
    }

    /// Move every block in the Visual range up among its siblings,
    /// keeping the range selected so the user can repeat. Ascending
    /// order: the top block slides up past the block above the range,
    /// then each following block moves into the slot the previous one
    /// vacated, so the whole range shifts up by one.
    pub(crate) fn move_up_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        // Bail before the expensive snapshot + save when the leading
        // block is already the first sibling: the whole range is blocked,
        // and `Alt+↑` at the top edge shouldn't churn undo/redo or disk.
        let Some(lead) = path_for_index(&self.page.blocks, lo) else {
            return;
        };
        if lead.last() == Some(&0) {
            return;
        }
        self.snapshot_for_undo();
        for idx in lo..=hi {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                let _ = move_up_at_path(&mut self.page.blocks, &path);
            }
        }
        self.mode = Mode::Visual {
            anchor: lo.saturating_sub(1),
        };
        self.selected = hi.saturating_sub(1);
        self.save();
    }

    /// Move every block in the Visual range down among its siblings.
    /// Descending order (mirror of `move_up_visual_range`): the bottom
    /// block slides down past the block below the range first.
    pub(crate) fn move_down_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        // Same no-op guard as `move_up`, on the trailing edge: if the
        // bottom block is already the last sibling the range can't move.
        let Some(trail) = path_for_index(&self.page.blocks, hi) else {
            return;
        };
        if !self.has_next_sibling(&trail) {
            return;
        }
        self.snapshot_for_undo();
        for idx in (lo..=hi).rev() {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                let _ = move_down_at_path(&mut self.page.blocks, &path);
            }
        }
        let max_idx = flat_count(&self.page.blocks).saturating_sub(1);
        self.mode = Mode::Visual {
            anchor: (lo + 1).min(max_idx),
        };
        self.selected = (hi + 1).min(max_idx);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;
    use outl_core::workspace::Workspace;
    use outl_md::parse::OutlineNode;
    use tempfile::TempDir;

    fn leaf(text: &str) -> OutlineNode {
        OutlineNode {
            text: text.into(),
            ..Default::default()
        }
    }

    /// App seeded with four top-level sibling blocks (a, b, c, d) and a
    /// Visual range over `anchor..=selected`.
    fn app_with_range(anchor: usize, selected: usize) -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        app.page.blocks = vec![leaf("a"), leaf("b"), leaf("c"), leaf("d")];
        app.flat_len = 4;
        app.selected = selected;
        app.mode = Mode::Visual { anchor };
        (app, dir)
    }

    fn texts(app: &App) -> Vec<String> {
        app.page.blocks.iter().map(|b| b.text.clone()).collect()
    }

    #[test]
    fn move_up_slides_the_whole_range_past_the_block_above() {
        // Range {b, c}; the block above (a) drops below the range.
        let (mut app, _dir) = app_with_range(1, 2);
        app.move_up_visual_range();
        assert_eq!(texts(&app), ["b", "c", "a", "d"]);
        // Selection follows the range up one row.
        assert_eq!(app.visual_range(), Some((0, 1)));
    }

    #[test]
    fn move_down_slides_the_whole_range_past_the_block_below() {
        // Range {b, c}; the block below (d) rises above the range.
        let (mut app, _dir) = app_with_range(1, 2);
        app.move_down_visual_range();
        assert_eq!(texts(&app), ["a", "d", "b", "c"]);
        assert_eq!(app.visual_range(), Some((2, 3)));
    }

    #[test]
    fn move_up_is_a_no_op_when_the_range_is_already_at_the_top() {
        // Range {a, b}; nothing above to slide past — order + selection
        // stay put rather than scrambling the range against itself.
        let (mut app, _dir) = app_with_range(0, 1);
        app.move_up_visual_range();
        assert_eq!(texts(&app), ["a", "b", "c", "d"]);
        assert_eq!(app.visual_range(), Some((0, 1)));
        // The no-op bails before snapshotting, so it never churns undo.
        assert!(app.undo.is_empty());
    }

    #[test]
    fn move_down_is_a_no_op_when_the_range_is_already_at_the_bottom() {
        let (mut app, _dir) = app_with_range(2, 3);
        app.move_down_visual_range();
        assert_eq!(texts(&app), ["a", "b", "c", "d"]);
        assert_eq!(app.visual_range(), Some((2, 3)));
        assert!(app.undo.is_empty());
    }
}
