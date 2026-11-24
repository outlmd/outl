//! Which block does a chord in the backlinks panel act on.
//!
//! One owner for that question. The cursor in the panel is a *position*
//! (`Focus::Backlink { idx, node }`) and a mutation needs a pair of
//! [`NodeId`]s, so something has to turn one into the other; two
//! answers to that is two places that can disagree about which page a
//! keystroke writes to.
//!
//! It deliberately does **not** check that the block is live. Whether a
//! `NodeId` names a node in the tree is the backend's answer, given at
//! mutation time; asking it here too would be a second opinion on a
//! question whose answer changes between the two calls.

use outl_actions::Backlink;
use outl_core::id::NodeId;

use crate::actions::node_id;
use crate::state::{App, Focus};

/// The page and the block a backlink row names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BacklinkTarget {
    /// Root of the page the citing block lives on.
    pub(crate) page: NodeId,
    /// The citing block itself.
    pub(crate) block: NodeId,
}

impl App {
    /// The page and block the cursor names, or `None` when it names
    /// nothing a mutation can be keyed on:
    ///
    /// - the cursor is in the outline, not the panel;
    /// - `idx` is past the end of the list (it is positional, and a
    ///   background rebuild can re-sort or shrink the list under it —
    ///   `None` is what stops the next chord landing on a different
    ///   page's block);
    /// - the backlink is an orphan (`source_page: None`), so there is
    ///   no page to commit against;
    /// - either id is not a ULID.
    pub(crate) fn focused_backlink_target(&self) -> Option<BacklinkTarget> {
        let Focus::Backlink { idx, node } = &self.focus else {
            return None;
        };
        let backlinks = self.backlinks_for_current();
        let bl = backlinks.get(*idx)?;
        target_of(bl, self.backlink_block_of(bl, *node)?)
    }

    /// [`Self::focused_backlink_target`], with the refusal said out loud.
    ///
    /// Nine chords in the panel need the pair, and every `None` above is
    /// reachable from a keystroke: an orphan row, an `idx` a background
    /// rebuild left behind, or a cross-page edit still in flight after
    /// the cursor went back to the outline. A chord that does nothing
    /// and says nothing reads as a dropped keypress, so the sentence
    /// lives here once rather than in nine branches.
    pub(crate) fn focused_backlink_target_or_report(&mut self) -> Option<BacklinkTarget> {
        let target = self.focused_backlink_target();
        if target.is_none() {
            self.status = "this backlink names no block to edit".into();
        }
        target
    }
}

/// The pure half: the page a row lives on plus the block the cursor
/// names in it (the citing block or one of its descendants, same page),
/// or `None` when the row has no usable page.
fn target_of(bl: &Backlink, block: NodeId) -> Option<BacklinkTarget> {
    Some(BacklinkTarget {
        page: node_id(&bl.source_page.as_ref()?.id)?,
        block,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::block::test_support::{
        workspace_with_a_backlink, workspace_with_backlinks_on_two_pages,
        workspace_with_two_backlinks,
    };
    use outl_actions::OutlineNode;

    /// An orphan row cannot be built through the real index — both
    /// builders always carry a `PageMeta` — so this one is assembled by
    /// hand to exercise the refusal.
    fn orphan_backlink() -> Backlink {
        Backlink {
            block_id: NodeId::new().to_string(),
            block_text: "mentions [[target]]".into(),
            todo: None,
            source_page: None,
            source_block: OutlineNode {
                id: NodeId::new().to_string(),
                text: "mentions [[target]]".into(),
                todo: None,
                collapsed: false,
                properties: Vec::new(),
                tokens: Vec::new(),
                table: None,
                children: Vec::new(),
            },
            source_block_path: Vec::new(),
            ancestors: Vec::new(),
            source_path: None,
        }
    }

    #[test]
    fn it_resolves_the_cited_block_and_its_page() {
        let (app, _d, _source) = workspace_with_a_backlink();

        let target = app
            .focused_backlink_target()
            .expect("a healthy backlink row resolves");

        let backlinks = app.backlinks_for_current();
        let bl = &backlinks[0];
        assert_eq!(
            target.block.to_string(),
            bl.block_id,
            "the block is the citing block the row renders"
        );
        assert_eq!(
            target.page.to_string(),
            bl.source_page.as_ref().unwrap().id,
            "the page is the one the citing block lives on, not the open page"
        );
        assert_eq!(
            Some(target.page),
            outl_actions::find_by_slug(&app.workspace, "source"),
            "and it resolves to `source`, the page off screen"
        );
    }

    #[test]
    fn an_orphan_backlink_resolves_to_nothing() {
        // No `source_page` means no page to commit against. Returning
        // the block alone would hand a mutation half a key.
        let bl = orphan_backlink();
        let block = node_id(&bl.block_id).unwrap();
        assert_eq!(target_of(&bl, block), None);
    }

    #[test]
    fn an_index_past_the_end_of_the_list_resolves_to_nothing() {
        // The list can re-sort or shrink under a background rebuild,
        // and `idx` is positional. `None` is what stops the next chord
        // landing on a different page's block.
        let (mut app, _d, _source) = workspace_with_a_backlink();
        app.focus = Focus::Backlink { idx: 7, node: None };

        assert_eq!(app.focused_backlink_target(), None);
    }

    #[test]
    fn the_cursor_in_the_outline_names_no_backlink() {
        let (mut app, _d, _source) = workspace_with_a_backlink();
        app.focus = Focus::Outline;

        assert_eq!(app.focused_backlink_target(), None);
    }

    /// **The pin under the whole change, reversed on purpose.** A
    /// backlink's `source_block` still arrives shallow (#179: the TUI
    /// clones the whole list every frame, and a deep copy per frame is
    /// the failure shape of root invariant 15), but the children are
    /// no longer unreachable: they come from `App::backlink_subtree`,
    /// the index's side table, as a shared `Arc`.
    ///
    /// Both halves are one argument. The first is read from the
    /// workspace tree so the fixture cannot quietly lose its child;
    /// the second asserts the list is shallow **and** that the child is
    /// reachable through the accessor, and that two reads share one
    /// allocation, which is what "a refcount bump, not a copy" means.
    #[test]
    fn a_backlinks_source_block_stays_shallow_and_the_subtree_comes_from_the_accessor() {
        let (app, _d) = workspace_with_two_backlinks();
        let backlinks = app.backlinks_for_current();
        assert_eq!(backlinks.len(), 2, "setup: two citing blocks");

        let mut rows_with_children = 0;
        for bl in &backlinks {
            assert!(
                bl.source_block.children.is_empty(),
                "the list the TUI clones per frame must stay shallow (row {:?})",
                bl.source_block.text
            );
            let in_tree = node_id(&bl.block_id)
                .map(|id| !outl_actions::children_of(&app.workspace, id).is_empty())
                .unwrap_or(false);
            let sub = app.backlink_subtree(&bl.block_id);
            assert_eq!(
                in_tree,
                sub.is_some(),
                "the accessor must agree with the tree about row {:?}",
                bl.source_block.text
            );
            if let Some(first) = sub {
                rows_with_children += 1;
                let again = app.backlink_subtree(&bl.block_id).unwrap();
                assert!(
                    std::sync::Arc::ptr_eq(&first, &again),
                    "two reads must share one allocation, not deep-copy"
                );
            }
        }
        assert_eq!(rows_with_children, 1, "setup: exactly one row has a child");
    }

    /// With no children under a row, `j` and `k` step across the list
    /// exactly as before and never produce a `node`.
    #[test]
    fn j_from_a_childless_backlink_lands_on_the_next_backlink() {
        let (mut app, _d) = workspace_with_backlinks_on_two_pages();
        assert_eq!(app.focus, Focus::Backlink { idx: 0, node: None });

        app.move_selection(1);

        assert_eq!(app.focus, Focus::Backlink { idx: 1, node: None });
        let landed = app.current_block_text();
        assert!(landed.contains("[[target]]"), "landed on {landed:?}");
    }

    /// `k` is the same walk backwards, ending in the outline.
    #[test]
    fn k_from_the_second_childless_backlink_lands_on_the_first() {
        let (mut app, _d) = workspace_with_backlinks_on_two_pages();
        app.focus = Focus::Backlink { idx: 1, node: None };

        app.move_selection(-1);

        assert_eq!(app.focus, Focus::Backlink { idx: 0, node: None });

        app.move_selection(-1);

        assert!(
            matches!(app.focus, Focus::Outline),
            "topping out of the panel returns to the outline"
        );
    }
}
