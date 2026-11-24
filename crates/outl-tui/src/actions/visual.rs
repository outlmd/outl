//! Visual mode: a Normal-mode sibling that adds a range anchor so the
//! user can act on N blocks at once (delete, indent, outdent).

use crate::outline_ops::{
    delete_at_path, flat_count, indent_at_path, index_for_path, outdent_at_path, path_for_index,
    Direction,
};
use crate::state::{App, Focus, Mode};

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

    /// Move every block in the Visual range up among its siblings,
    /// keeping the range selected so the user can repeat.
    pub(crate) fn move_up_visual_range(&mut self) {
        self.move_visual_range(Direction::Up);
    }

    /// Move every block in the Visual range down among its siblings.
    pub(crate) fn move_down_visual_range(&mut self) {
        self.move_visual_range(Direction::Down);
    }

    /// Rotate the range as one unit through [`App::rotate_run`].
    ///
    /// A range holding a parent and its children has one root, the
    /// parent, so the subtree travels whole. Roots with different parents
    /// (a range that starts on a child and runs past its parent's end)
    /// refuse rather than scramble. The anchor and the cursor are found
    /// again by path, so the same blocks stay selected whichever end the
    /// cursor was on.
    fn move_visual_range(&mut self, dir: Direction) {
        if !matches!(self.focus, Focus::Outline) {
            return;
        }
        let (Some((lo, hi)), Mode::Visual { anchor }) = (self.visual_range(), &self.mode) else {
            return;
        };
        let anchor = *anchor;
        let Some(paths) = (lo..=hi)
            .map(|i| path_for_index(&self.page.blocks, i))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let Some((parent, first, last)) = range_roots(&paths) else {
            return;
        };
        let (Some(anchor_path), Some(cursor_path)) = (
            path_for_index(&self.page.blocks, anchor),
            path_for_index(&self.page.blocks, self.selected),
        ) else {
            return;
        };
        if !self.rotate_run(&parent, first, last, dir) {
            return;
        }
        let depth = parent.len();
        let shifted = |mut path: Vec<usize>| {
            if let Some(at) = path.get_mut(depth) {
                *at = match dir {
                    Direction::Up => at.saturating_sub(1),
                    Direction::Down => *at + 1,
                };
            }
            index_for_path(&self.page.blocks, &path)
        };
        let (anchor, cursor) = (shifted(anchor_path), shifted(cursor_path));
        self.mode = Mode::Visual {
            anchor: anchor.unwrap_or(lo),
        };
        self.selected = cursor.unwrap_or(hi);
        self.save();
    }
}

/// The sibling run a Visual range moves: `(parent, first, last)`.
///
/// Paths under another selected path ride along with it and are dropped.
/// `None` unless the surviving roots share one parent.
fn range_roots(paths: &[Vec<usize>]) -> Option<(Vec<usize>, usize, usize)> {
    let mut roots = paths
        .iter()
        .filter(|p| !paths.iter().any(|q| q.len() < p.len() && p.starts_with(q)));
    let (&first, parent) = roots.next()?.split_last()?;
    let (mut lo, mut hi) = (first, first);
    for path in roots {
        let (&idx, under) = path.split_last()?;
        if under != parent {
            return None;
        }
        lo = lo.min(idx);
        hi = hi.max(idx);
    }
    Some((parent.to_vec(), lo, hi))
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
        let mut app = App::new_for_tests(
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

    // ---- range moves as one unit ----

    use crate::input::handle_visual_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    const CTRL_SHIFT: KeyModifiers = KeyModifiers::CONTROL.union(KeyModifiers::SHIFT);

    fn node(text: &str, children: Vec<OutlineNode>) -> OutlineNode {
        OutlineNode {
            text: text.into(),
            children,
            ..Default::default()
        }
    }

    /// App over `blocks` with a Visual range `anchor..=selected` (flat).
    fn app_over(blocks: Vec<OutlineNode>, anchor: usize, selected: usize) -> (App, TempDir) {
        let (mut app, dir) = app_with_range(anchor, selected);
        app.flat_len = crate::outline_ops::flat_count(&blocks);
        app.page.blocks = blocks;
        (app, dir)
    }

    fn flat_texts(blocks: &[OutlineNode]) -> Vec<String> {
        blocks
            .iter()
            .flat_map(|b| {
                let mut v = vec![b.text.clone()];
                v.extend(flat_texts(&b.children));
                v
            })
            .collect()
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        handle_visual_key(app, KeyEvent::new(code, mods)).expect("key");
    }

    /// Run `f`, assert the page, mode, selection, undo and redo are as before.
    fn assert_untouched(mut app: App, f: impl FnOnce(&mut App)) {
        let before = (
            flat_texts(&app.page.blocks),
            app.visual_range(),
            app.selected,
        );
        let zoom = app.zoom_stack.clone();
        let (undo, redo) = (app.undo.len(), app.redo.len());
        f(&mut app);
        assert_eq!(flat_texts(&app.page.blocks), before.0);
        assert_eq!((app.visual_range(), app.selected), (before.1, before.2));
        assert_eq!(app.zoom_stack, zoom);
        assert_eq!((app.undo.len(), app.redo.len()), (undo, redo));
    }

    fn tree() -> Vec<OutlineNode> {
        // flat: a=0 p=1 c1=2 c2=3 d=4
        vec![
            leaf("a"),
            node("p", vec![leaf("c1"), leaf("c2")]),
            leaf("d"),
        ]
    }

    #[test]
    fn a_parent_and_its_children_move_as_one_unit() {
        let (mut app, _d) = app_over(tree(), 1, 3);
        app.move_up_visual_range();
        assert_eq!(flat_texts(&app.page.blocks), ["p", "c1", "c2", "a", "d"]);
        assert_eq!(app.visual_range(), Some((0, 2)));
        app.move_down_visual_range();
        app.move_down_visual_range();
        assert_eq!(flat_texts(&app.page.blocks), ["a", "d", "p", "c1", "c2"]);
        assert_eq!(app.visual_range(), Some((2, 4)));
    }

    #[test]
    fn the_range_passes_a_displaced_sibling_with_its_children() {
        let blocks = vec![node("a", vec![leaf("x"), leaf("y")]), leaf("b")];
        let (mut app, _d) = app_over(blocks, 3, 3);
        app.move_up_visual_range();
        assert_eq!(flat_texts(&app.page.blocks), ["b", "a", "x", "y"]);
        assert_eq!(app.visual_range(), Some((0, 0)));
        app.move_down_visual_range();
        assert_eq!(flat_texts(&app.page.blocks), ["a", "x", "y", "b"]);
        assert_eq!(app.visual_range(), Some((3, 3)));
    }

    #[test]
    fn the_cursor_end_survives_a_move() {
        // Cursor above the anchor: still above it afterwards.
        let (mut app, _d) = app_over(tree(), 3, 1);
        app.move_up_visual_range();
        assert!(matches!(app.mode, Mode::Visual { anchor: 2 }));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_range_at_an_edge_changes_nothing_and_keeps_redo() {
        for (anchor, sel, up) in [(0, 0, true), (4, 4, false)] {
            let (mut app, _d) = app_over(tree(), anchor, sel);
            app.redo.push(crate::state::HistorySnapshot {
                page: app.page.clone(),
                selected: 0,
                cursor_col: 0,
                view_path: app.current_path(),
            });
            assert_untouched(app, |app| {
                if up {
                    app.move_up_visual_range();
                } else {
                    app.move_down_visual_range();
                }
            });
        }
    }

    #[test]
    fn roots_with_different_parents_refuse() {
        // x (child of a) .. b (top level)
        let blocks = vec![node("a", vec![leaf("x"), leaf("y")]), leaf("b")];
        for (anchor, sel) in [(1, 3), (2, 3), (1, 3)] {
            let (app, _d) = app_over(blocks.clone(), anchor, sel);
            assert_untouched(app, |app| {
                app.move_up_visual_range();
                app.move_down_visual_range();
            });
        }
    }

    #[test]
    fn a_range_holding_the_zoom_root_refuses() {
        let blocks = vec![leaf("a"), leaf("b"), leaf("c")];
        let (mut app, _d) = app_over(blocks, 0, 1);
        app.zoom_stack = vec![vec![1]];
        assert_untouched(app, |app| {
            app.move_up_visual_range();
            app.move_down_visual_range();
        });
    }

    #[test]
    fn children_of_the_zoom_root_move() {
        let blocks = vec![leaf("a"), node("z", vec![leaf("x"), leaf("y")])];
        let (mut app, _d) = app_over(blocks, 3, 3);
        app.zoom_stack = vec![vec![1]];
        app.move_up_visual_range();
        assert_eq!(flat_texts(&app.page.blocks), ["a", "z", "y", "x"]);
    }

    #[test]
    fn the_same_blocks_stay_selected_while_repeating() {
        let (mut app, _d) = app_with_range(2, 3);
        for _ in 0..2 {
            press(&mut app, KeyCode::Char('K'), KeyModifiers::SHIFT);
        }
        assert_eq!(texts(&app), ["c", "d", "a", "b"]);
        assert_eq!(app.visual_range(), Some((0, 1)));
        press(&mut app, KeyCode::Char('K'), KeyModifiers::SHIFT);
        assert_eq!(texts(&app), ["c", "d", "a", "b"], "edge: unchanged");
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('J'), KeyModifiers::SHIFT);
        }
        assert_eq!(texts(&app), ["a", "b", "c", "d"]);
        assert_eq!(app.visual_range(), Some((2, 3)));
    }

    #[test]
    fn every_move_chord_drives_the_range_and_plain_arrows_extend() {
        for mods in [KeyModifiers::ALT, CTRL_SHIFT] {
            let (mut app, _d) = app_with_range(1, 2);
            press(&mut app, KeyCode::Up, mods);
            assert_eq!(texts(&app), ["b", "c", "a", "d"], "{mods:?} up");
            press(&mut app, KeyCode::Down, mods);
            assert_eq!(texts(&app), ["a", "b", "c", "d"], "{mods:?} down");
        }
        let (mut app, _d) = app_with_range(1, 1);
        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(texts(&app), ["a", "b", "c", "d"]);
        assert_eq!(app.visual_range(), Some((1, 2)));
        for mods in [KeyModifiers::SHIFT, KeyModifiers::CONTROL] {
            press(&mut app, KeyCode::Down, mods);
            assert_eq!(texts(&app), ["a", "b", "c", "d"], "{mods:?} must not move");
        }
    }

    #[test]
    fn nothing_moves_with_the_backlink_panel_focused() {
        let (mut app, _d) = app_with_range(1, 2);
        app.focus = Focus::Backlink { idx: 0, node: None };
        assert_untouched(app, |app| {
            app.move_up_visual_range();
            app.move_down_visual_range();
        });
    }

    #[test]
    fn range_roots_drops_descendants_and_needs_one_parent() {
        let p = |v: &[usize]| v.to_vec();
        assert_eq!(
            range_roots(&[p(&[1]), p(&[1, 0]), p(&[1, 1]), p(&[2])]),
            Some((vec![], 1, 2))
        );
        assert_eq!(
            range_roots(&[p(&[0, 1]), p(&[0, 2])]),
            Some((p(&[0]), 1, 2))
        );
        assert_eq!(range_roots(&[p(&[0, 1]), p(&[1])]), None);
        assert_eq!(range_roots(&[]), None);
    }
}
