//! `g o`: open the page a backlink lives on, at the block above it.
//!
//! The backlink panel is the one surface where the row's block cannot be
//! reached by any other key (the header has no `block_starts` entry and
//! `Enter` edits), so this chord is its only door to the source page.
//!
//! Order matters: the fold is an op, so it is written **before** the
//! page loads. `load_current` rehydrates `collapsed` from the tree and
//! `id_by_flat` from the sidecar, and only then can the selection be
//! found by id. Selecting by a path computed before the unfold would
//! land on a row that is still hidden (`index_for_path` ignores folds).

use crate::actions::node_id;
use crate::state::{App, Focus, View};
use outl_actions::{
    ancestors_below_page, enclosing_page_id, find_by_slug, is_live, refusal, unfold_ancestors,
    PageKind, PageMeta,
};

impl App {
    pub(crate) fn open_backlink_context(&mut self) {
        let Focus::Backlink { idx, .. } = self.focus else {
            self.status = "g o opens a backlink's page: move into the backlinks first".into();
            return;
        };
        // Read the row the user is on *before* the flush: a pending edit
        // can re-index the panel, and `idx` would then name another row.
        let Some(bl) = self.backlinks_for_current().get(idx).cloned() else {
            self.status = "that backlink is gone: nothing to open".into();
            return;
        };
        self.flush_pending_save();
        let Some(page) = bl.source_page.clone() else {
            self.status = "that backlink has no page to open".into();
            return;
        };
        let Some(page_id) = find_by_slug(&self.workspace, &page.slug) else {
            self.status = format!("{} is no longer in the workspace", page.title);
            return;
        };
        let citing = node_id(&bl.block_id);
        let crumb = bl.ancestors.last().and_then(|c| node_id(&c.id));

        // A peer may have moved either block to another page since the
        // index was built; unfolding there would write ops on a page the
        // user never asked about. Same gate as the Tauri command's
        // `row_is_live_on`.
        let on_page = |ws, n| is_live(ws, n) && enclosing_page_id(ws, n) == Some(page_id);
        let citing = citing.filter(|c| on_page(&self.workspace, *c));
        let crumb = crumb.filter(|c| on_page(&self.workspace, *c));

        // Unfold from the citing block; if it is gone, from its parent.
        let unfold_failed = match citing.or(crumb) {
            Some(n) => unfold_ancestors(&mut self.workspace, &self.hlc, n).is_err(),
            None => false,
        };

        self.open_source_page(&page);

        let citing_chain = citing.map(|c| ancestors_below_page(&self.workspace, c));
        let citing_parent = citing_chain.as_ref().and_then(|c| c.first().copied());
        let row = [crumb, citing_parent]
            .into_iter()
            .flatten()
            .filter_map(|id| self.id_by_flat.iter().position(|x| *x == id))
            .find(|r| !self.hidden_by_collapse.get(*r).copied().unwrap_or(false));
        if let Some(row) = row {
            self.selected = row;
            self.cursor_col = 0;
            self.status = format!("opened {} at its context", page.title);
            return;
        }
        // A top-level citing block has no parent to land on and is not
        // stale: the page itself is the context.
        let top_level = citing_chain.is_some_and(|c| c.is_empty());
        self.status = if top_level {
            format!("opened {}", page.title)
        } else if unfold_failed {
            format!(
                "Could not open the folded blocks above that backlink. Opened {} instead.",
                page.title
            )
        } else {
            let gone = node_id(&bl.block_id)
                .map_or("That block could not be found, so nothing changed.", |c| {
                    refusal::stale_backlink_block_unchanged_refusal(&self.workspace, c)
                });
            format!("{gone} Opened {} instead.", page.title)
        };
    }

    /// Switch to `page`. The caller has already checked the page node
    /// exists, so a missing `.md` is a sidecar-only page, not a stale
    /// backlink to be minted into an empty one.
    fn open_source_page(&mut self, page: &PageMeta) {
        let view = match page.kind {
            PageKind::Journal => chrono::NaiveDate::parse_from_str(&page.slug, "%Y-%m-%d")
                .ok()
                .map(View::Journal),
            PageKind::Page => None,
        }
        .unwrap_or_else(|| {
            View::Page(
                self.workspace_root
                    .join("pages")
                    .join(format!("{}.md", page.slug)),
            )
        });
        self.view = view;
        self.selected = 0;
        self.cursor_col = 0;
        self.load_current();
    }
}

#[cfg(test)]
mod tests {
    use crate::actions::block::test_support::open_slugs;
    use crate::state::Focus;
    use outl_actions::{ancestors_below_page, set_block_collapsed};

    const SOURCE: &str = "- grand\n  - parent\n    - mentions [[target]]\n- other\n";

    fn collapsed_fixture() -> (
        crate::state::App,
        tempfile::TempDir,
        outl_core::id::NodeId,
        outl_core::id::NodeId,
    ) {
        let (mut app, dir) = open_slugs(&[("target", "- t\n"), ("source", SOURCE)], "target");
        let bl = app.backlinks_for_current().remove(0);
        let citing = crate::actions::node_id(&bl.block_id).unwrap();
        let chain = ancestors_below_page(&app.workspace, citing);
        assert_eq!(chain.len(), 2, "setup: grand > parent > citing");
        for id in &chain {
            set_block_collapsed(&mut app.workspace, &app.hlc, *id, true).unwrap();
        }
        (app, dir, chain[0], chain[1])
    }

    #[test]
    fn g_o_unfolds_the_chain_and_selects_the_parent() {
        let (mut app, _dir, parent, grand) = collapsed_fixture();
        app.open_backlink_context();

        assert!(!app.workspace.tree().is_collapsed(parent));
        assert!(!app.workspace.tree().is_collapsed(grand));
        assert_eq!(app.current_slug(), "source");
        assert_eq!(app.focus, Focus::Outline);
        assert_eq!(app.id_by_flat[app.selected], parent);
        assert!(
            !app.hidden_by_collapse[app.selected],
            "the selected row must be visible"
        );
    }

    #[test]
    fn g_o_outside_the_backlink_panel_changes_nothing() {
        let (mut app, _dir, parent, _grand) = collapsed_fixture();
        app.focus = Focus::Outline;
        app.open_backlink_context();
        assert_eq!(app.current_slug(), "target");
        assert!(app.workspace.tree().is_collapsed(parent));
    }

    #[test]
    fn a_top_level_citing_block_opens_the_page_and_writes_nothing() {
        let (mut app, _dir) = open_slugs(
            &[("target", "- t\n"), ("source", "- mentions [[target]]\n")],
            "target",
        );
        app.open_backlink_context();
        assert_eq!(app.current_slug(), "source");
        assert_eq!(app.focus, Focus::Outline);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_vanished_citing_block_still_opens_the_page() {
        let (mut app, _dir, _parent, _grand) = collapsed_fixture();
        let bl = app.backlinks_for_current().remove(0);
        let citing = crate::actions::node_id(&bl.block_id).unwrap();
        outl_actions::delete(&mut app.workspace, &app.hlc, citing).ok();
        app.open_backlink_context();
        assert_eq!(app.current_slug(), "source");
        assert_eq!(app.focus, Focus::Outline);
    }

    fn press(app: &mut crate::state::App, c: char) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        crate::input::handle_normal_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }

    #[test]
    fn the_g_o_keys_reach_the_handler_in_the_backlink_panel() {
        let (mut app, _dir, _parent, _grand) = collapsed_fixture();
        press(&mut app, 'g');
        press(&mut app, 'o');
        assert_eq!(app.current_slug(), "source");
        assert_eq!(app.focus, Focus::Outline);
    }

    #[test]
    fn the_g_o_keys_outside_the_panel_only_say_why() {
        let (mut app, _dir, parent, _grand) = collapsed_fixture();
        app.focus = Focus::Outline;
        press(&mut app, 'g');
        press(&mut app, 'o');
        assert_eq!(app.current_slug(), "target");
        assert!(app.workspace.tree().is_collapsed(parent));
        assert!(app.status.contains("backlinks"), "{}", app.status);
    }

    #[test]
    fn a_citing_block_moved_to_another_page_is_not_unfolded_there() {
        let (mut app, _dir) = open_slugs(
            &[
                ("target", "- t\n"),
                ("source", SOURCE),
                ("elsewhere", "- home\n  - kid\n"),
            ],
            "target",
        );
        let bl = app.backlinks_for_current().remove(0);
        let citing = crate::actions::node_id(&bl.block_id).unwrap();
        let elsewhere = outl_actions::find_by_slug(&app.workspace, "elsewhere").unwrap();
        let home = outl_actions::children_of(&app.workspace, elsewhere)[0].0;
        let kid = outl_actions::children_of(&app.workspace, home)[0].0;
        set_block_collapsed(&mut app.workspace, &app.hlc, home, true).unwrap();
        outl_actions::move_under(&mut app.workspace, &app.hlc, citing, kid).unwrap();

        app.open_backlink_context();

        assert!(
            app.workspace.tree().is_collapsed(home),
            "a page the user did not ask about must not be written"
        );
    }
}
