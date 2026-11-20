//! Which block does a chord in the backlinks panel act on.
//!
//! One owner for that question. The cursor in the panel is a *position*
//! (`Focus::Backlink { idx }`) and a mutation needs a pair of
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
        let Focus::Backlink { idx } = &self.focus else {
            return None;
        };
        target_of(self.backlinks_for_current().get(*idx)?)
    }

    /// Which row names `block`, or `None` when the list no longer holds
    /// it.
    ///
    /// The inverse of [`Self::focused_backlink_target`], here for the
    /// same reason: the panel's cursor is a *position* and a mutation is
    /// keyed on a [`NodeId`], so both directions of that mapping belong
    /// to one module.
    ///
    /// The caller is the Insert re-entry after a cross-page structural
    /// op (`actions::block::structural`). It asked `Focus::Backlink` and
    /// nothing else, and the commit in between can re-index the source
    /// page and re-clamp `idx` — so a positional re-entry put the caret
    /// on a different citing block, sometimes on a different page, with
    /// nothing on screen to say the target had changed.
    pub(crate) fn backlink_row_of(&self, block: NodeId) -> Option<usize> {
        self.backlinks_for_current()
            .iter()
            .position(|bl| node_id(&bl.block_id) == Some(block))
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

/// The pure half: a row's ids, or `None` when it has no usable pair.
fn target_of(bl: &Backlink) -> Option<BacklinkTarget> {
    Some(BacklinkTarget {
        page: node_id(&bl.source_page.as_ref()?.id)?,
        block: node_id(&bl.block_id)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::block::test_support::{
        workspace_with_a_backlink, workspace_with_two_backlinks,
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
        assert_eq!(target_of(&orphan_backlink()), None);
    }

    #[test]
    fn an_index_past_the_end_of_the_list_resolves_to_nothing() {
        // The list can re-sort or shrink under a background rebuild,
        // and `idx` is positional. `None` is what stops the next chord
        // landing on a different page's block.
        let (mut app, _d, _source) = workspace_with_a_backlink();
        app.focus = Focus::Backlink { idx: 7 };

        assert_eq!(app.focused_backlink_target(), None);
    }

    #[test]
    fn the_cursor_in_the_outline_names_no_backlink() {
        let (mut app, _d, _source) = workspace_with_a_backlink();
        app.focus = Focus::Outline;

        assert_eq!(app.focused_backlink_target(), None);
    }

    /// **The pin under the whole change.** `sub_path` was removed
    /// because a backlink's `source_block` arrives shallow, which is
    /// what made the subtree branch in `nav::selection` unreachable.
    /// That is a property of the *index builders*, not of this crate,
    /// so nothing else in the diff fails if it stops being true: revert
    /// #179 and the removal is wrong in hindsight with every other test
    /// still green.
    ///
    /// The two halves are **one argument**, and neither stands alone:
    /// *this citing block has a child, and the backlink's view of it
    /// does not carry it.* The first half is read from the workspace
    /// tree (`children_of`), not from the fixture's text, because a
    /// shallow-`source_block` assertion is trivially true for a row
    /// that has no child to drop — so without it, simplifying
    /// `test_support.rs` would disarm this pin with every test still
    /// green, which is the failure mode one level up from the one the
    /// pin exists to catch.
    ///
    /// Asserting over every row rather than row 0 alone, because the
    /// sort order is not this test's business.
    #[test]
    fn a_backlinks_source_block_arrives_shallow() {
        let (app, _d) = workspace_with_two_backlinks();
        let backlinks = app.backlinks_for_current();
        assert_eq!(
            backlinks.len(),
            2,
            "setup: two citing blocks, the first with a child on disk"
        );

        // Half one: at least one citing block really does have a child,
        // observed in the tree rather than assumed from the fixture.
        let with_a_child: Vec<&str> = backlinks
            .iter()
            .filter(|bl| {
                node_id(&bl.block_id)
                    .map(|id| !outl_actions::children_of(&app.workspace, id).is_empty())
                    .unwrap_or(false)
            })
            .map(|bl| bl.source_block.text.as_str())
            .collect();
        assert!(
            !with_a_child.is_empty(),
            "the fixture must give this pin something to catch: no citing \
             block has a child in the tree, which makes the shallow \
             assertion below true whatever the index builders do. Rows: {:?}",
            backlinks
                .iter()
                .map(|bl| &bl.source_block.text)
                .collect::<Vec<_>>()
        );

        // Half two: and the backlink's copy of it is shallow anyway.
        for bl in &backlinks {
            assert!(
                bl.source_block.children.is_empty(),
                "a backlink's source_block is shallow (#179: materializing \
                 every referencing block's subtree under the workspace lock \
                 is what froze input); `Focus::Backlink.sub_path` was removed \
                 on that basis, so a cursor into the subtree now has nowhere \
                 to point — row {:?} arrived with {} children, and {:?} \
                 do have children in the tree",
                bl.source_block.text,
                bl.source_block.children.len(),
                with_a_child
            );
        }
    }

    /// `sub_path` was always empty, so the subtree branch in
    /// `nav::selection` was unreachable and `j` has only ever stepped
    /// across the list. A regression net against a future change to
    /// `step_forward`; the claim that makes the removal correct is
    /// pinned by `a_backlinks_source_block_arrives_shallow` above.
    #[test]
    fn j_from_a_backlink_still_lands_on_the_next_backlink() {
        let (mut app, _d) = workspace_with_two_backlinks();
        let rows: Vec<String> = app
            .backlinks_for_current()
            .iter()
            .map(|bl| bl.source_block.text.clone())
            .collect();
        assert_eq!(rows.len(), 2, "setup: two citing blocks, one child");
        assert_eq!(app.focus, Focus::Backlink { idx: 0 });

        app.move_selection(1);

        assert_eq!(
            app.focus,
            Focus::Backlink { idx: 1 },
            "`j` steps to the next backlink, never into the first one's subtree"
        );
        let landed = app.current_block_text();
        assert!(
            landed.contains("[[target]]"),
            "landed on {landed:?}, which is not a citing block — the cursor \
             walked into the subtree instead of across the list"
        );
        assert_ne!(landed, rows[0], "and it is not the row it started on");
    }

    /// `k` is the same walk backwards, and the same proof: one row per
    /// backlink in both directions, with the outline below the top.
    #[test]
    fn k_from_the_second_backlink_lands_on_the_first() {
        let (mut app, _d) = workspace_with_two_backlinks();
        app.focus = Focus::Backlink { idx: 1 };

        app.move_selection(-1);

        assert_eq!(app.focus, Focus::Backlink { idx: 0 });

        app.move_selection(-1);

        assert!(
            matches!(app.focus, Focus::Outline),
            "topping out of the panel returns to the outline"
        );
    }
}
