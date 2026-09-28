//! Moving the selection from block to block, and where that lands in
//! the viewport.
//!
//! Block granularity only — the caret *inside* a block's text is
//! `super::cursor`. The flat list walked here is virtual: the outline's
//! blocks followed by the inline backlinks section, so `j` / `k` cross
//! that boundary without the caller knowing there is one.

use crate::state::{App, Focus};
use outl_actions::flatten_subtree_paths;

impl App {
    pub(crate) fn move_selection(&mut self, delta: i32) {
        if self.flat_len == 0 && matches!(self.focus, Focus::Outline) {
            self.selected = 0;
            self.cursor_col = 0;
            return;
        }
        if delta > 0 {
            for _ in 0..delta {
                if !self.step_forward() {
                    break;
                }
            }
        } else {
            for _ in 0..(-delta) {
                if !self.step_backward() {
                    break;
                }
            }
        }
    }

    /// Advance the cursor by one position in the virtual flat list
    /// `outline blocks ++ backlink section blocks`. Returns `true` if
    /// the cursor moved, `false` when already at the bottom.
    ///
    /// Crosses the boundary between outline and backlinks transparently
    /// when the inline section is shown and non-empty.
    fn step_forward(&mut self) -> bool {
        match self.focus.clone() {
            Focus::Outline => {
                // When zoomed into a block, navigation is confined to
                // that block's subtree window `[start, end)`; otherwise
                // the window is the whole page.
                let (_, end) = self.zoom_root_window();
                // Walk forward until we hit a visible block (not
                // hidden under a collapsed ancestor) or fall off the
                // end of the (zoom-confined) outline.
                let mut next = self.selected + 1;
                while next < end && self.hidden_by_collapse.get(next).copied().unwrap_or(false) {
                    next += 1;
                }
                if next < end {
                    self.selected = next;
                    self.cursor_col = 0;
                    return true;
                }
                // Bottom of outline → try entering the backlinks zone.
                // Only when the whole page is shown: a zoomed subtree
                // ends before `flat_len`, and its backlinks aren't part
                // of the focused view, so `j` stops at the subtree edge.
                if self.zoom_stack.is_empty() && self.backlinks_navigable() {
                    self.focus = Focus::Backlink {
                        idx: 0,
                        sub_path: Vec::new(),
                    };
                    self.cursor_col = 0;
                    return true;
                }
                false
            }
            Focus::Backlink { idx, sub_path } => {
                // Borrow the backlinks slice directly instead of
                // cloning the whole `Vec<Backlink>` (each entry
                // carries an `OutlineNode` subtree — non-trivial to
                // clone per keystroke).
                let slug = self.current_slug();
                let new_focus = {
                    let backlinks = self.backlinks_for_slug(&slug);
                    let Some(bl) = backlinks.get(idx) else {
                        return false;
                    };
                    let paths = flatten_subtree_paths(&bl.source_block);
                    let cur_pos = paths.iter().position(|p| p == &sub_path).unwrap_or(0);
                    if cur_pos + 1 < paths.len() {
                        Focus::Backlink {
                            idx,
                            sub_path: paths[cur_pos + 1].clone(),
                        }
                    } else if idx + 1 < backlinks.len() {
                        Focus::Backlink {
                            idx: idx + 1,
                            sub_path: Vec::new(),
                        }
                    } else {
                        return false;
                    }
                };
                self.focus = new_focus;
                self.cursor_col = 0;
                true
            }
        }
    }

    /// Mirror of [`step_forward`], moving one position upward.
    fn step_backward(&mut self) -> bool {
        match self.focus.clone() {
            Focus::Outline => {
                // The zoom root is the top of the confined window — `k`
                // must not walk above it. Not zoomed → floor is 0.
                let (start, _) = self.zoom_root_window();
                // Walk backward over hidden subtree entries the same
                // way `step_forward` skips them going down.
                if self.selected <= start {
                    return false;
                }
                let mut prev = self.selected - 1;
                while prev > start && self.hidden_by_collapse.get(prev).copied().unwrap_or(false) {
                    prev -= 1;
                }
                if self.hidden_by_collapse.get(prev).copied().unwrap_or(false) {
                    // Reached the top still inside a collapsed
                    // subtree — no visible previous block.
                    return false;
                }
                self.selected = prev;
                self.cursor_col = 0;
                true
            }
            Focus::Backlink { idx, sub_path } => {
                let slug = self.current_slug();
                // Resolve the new focus value while only borrowing the
                // backlinks slice — no `to_vec` clone per keystroke.
                let new_focus_opt = {
                    let backlinks = self.backlinks_for_slug(&slug);
                    let Some(bl) = backlinks.get(idx) else {
                        return false;
                    };
                    let paths = flatten_subtree_paths(&bl.source_block);
                    let cur_pos = paths.iter().position(|p| p == &sub_path).unwrap_or(0);
                    if cur_pos > 0 {
                        Some(Focus::Backlink {
                            idx,
                            sub_path: paths[cur_pos - 1].clone(),
                        })
                    } else if idx > 0 {
                        // Jump to the last block of the previous backlink.
                        let prev_paths = flatten_subtree_paths(&backlinks[idx - 1].source_block);
                        let last = prev_paths.last().cloned().unwrap_or_default();
                        Some(Focus::Backlink {
                            idx: idx - 1,
                            sub_path: last,
                        })
                    } else {
                        // Topping out → fall back into the outline.
                        None
                    }
                };
                match new_focus_opt {
                    Some(f) => self.focus = f,
                    None => {
                        self.focus = Focus::Outline;
                        self.selected = self.flat_len.saturating_sub(1);
                    }
                }
                self.cursor_col = 0;
                true
            }
        }
    }

    /// `true` when the inline backlinks section is rendered *and* has
    /// at least one block the cursor can land on. Drives the cross-zone
    /// transition in `step_forward`/`step_backward`.
    fn backlinks_navigable(&self) -> bool {
        self.show_backlinks && self.backlinks_count_for_current() > 0
    }

    /// `zz` — center the viewport vertically on the selected block.
    /// Adjusts `scroll_y` so the selection lands at the midpoint of
    /// `viewport_height`. Clamps at 0 so we don't scroll above the
    /// first block.
    pub(crate) fn center_viewport_on_selection(&mut self) {
        let vp = self.viewport_height.max(1) as i32;
        let target = (self.selected as i32) - vp / 2;
        self.scroll_y = target.max(0) as u16;
    }
}
