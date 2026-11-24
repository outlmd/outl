//! Moving the selection from block to block, and where that lands in
//! the viewport.
//!
//! Block granularity only — the caret *inside* a block's text is
//! `super::cursor`. The flat list walked here is virtual: the outline's
//! blocks followed by the inline backlinks section, so `j` / `k` cross
//! that boundary without the caller knowing there is one.

use crate::state::{App, Focus};

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
                // Only when the section is actually drawn: a zoom takes
                // it off screen (`page_sections_visible`, the one owner
                // the renderers ask too), so `j` stops at the subtree
                // edge instead of parking the cursor on an invisible row.
                if self.page_sections_visible() && self.backlinks_navigable() {
                    self.focus = Focus::Backlink { idx: 0, node: None };
                    self.cursor_col = 0;
                    return true;
                }
                false
            }
            Focus::Backlink { idx, node } => self.backlink_step_forward(idx, node),
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
            Focus::Backlink { idx, node } => self.backlink_step_backward(idx, node),
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

#[cfg(test)]
mod tests {
    use crate::state::{App, Focus, View};
    use outl_core::hlc::HlcGenerator;
    use outl_core::{ActorId, Workspace};
    use outl_md::reconcile::reconcile_md;
    use std::fs;
    use tempfile::TempDir;

    /// `pages/target.md` on screen, cited by two blocks of `pages/source.md`,
    /// with the real index built from disk (a faked list would not run
    /// the count the steps are gated on). Focus starts on the outline.
    fn target_cited_twice() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pages")).unwrap();
        let target = dir.path().join("pages").join("target.md");
        let source = dir.path().join("pages").join("source.md");
        fs::write(&target, "- target body\n").unwrap();
        fs::write(&source, "- first [[target]]\n- second [[target]]\n").unwrap();

        let actor = ActorId::new();
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        reconcile_md(&mut ws, &hlc, &target, None).unwrap();
        reconcile_md(&mut ws, &hlc, &source, None).unwrap();
        let metas: Vec<_> = ["target", "source"]
            .iter()
            .filter_map(|s| outl_actions::find_by_slug(&ws, s))
            .filter_map(|id| outl_actions::page_meta(&ws, id))
            .collect();
        let index = outl_actions::build_backlink_index_from_disk(&metas, dir.path());

        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        *app.backlink_index.borrow_mut() = Some(index);
        app.view = View::Page(target);
        app.load_current_no_autorun();
        app.show_backlinks = true;
        (app, dir)
    }

    #[test]
    fn j_walks_the_backlinks_and_stops_at_the_last_one() {
        let (mut app, _d) = target_cited_twice();
        assert_eq!(app.backlinks_for_current().len(), 2, "setup");
        app.selected = app.flat_len - 1;

        app.move_selection(1);
        assert_eq!(app.focus, Focus::Backlink { idx: 0, node: None });
        app.move_selection(1);
        assert_eq!(app.focus, Focus::Backlink { idx: 1, node: None });
        app.move_selection(1);
        assert_eq!(
            app.focus,
            Focus::Backlink { idx: 1, node: None },
            "no row past the end"
        );
    }

    #[test]
    fn k_walks_back_up_and_leaves_the_panel_into_the_last_block() {
        let (mut app, _d) = target_cited_twice();
        app.focus = Focus::Backlink { idx: 1, node: None };

        app.move_selection(-1);
        assert_eq!(app.focus, Focus::Backlink { idx: 0, node: None });
        app.move_selection(-1);
        assert_eq!(app.focus, Focus::Outline);
        assert_eq!(app.selected, app.flat_len - 1);
    }

    #[test]
    fn a_stale_index_past_the_end_moves_nowhere() {
        let (mut app, _d) = target_cited_twice();
        app.focus = Focus::Backlink { idx: 7, node: None };
        app.move_selection(1);
        assert_eq!(app.focus, Focus::Backlink { idx: 7, node: None });
        app.move_selection(-1);
        assert_eq!(app.focus, Focus::Backlink { idx: 7, node: None });
    }
}
