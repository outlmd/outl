//! Insert a new block relative to an existing one.
//!
//! The `create_before` / `create_after` pair and their
//! `*_or_append` fallbacks. Two concerns live here and nowhere else:
//!
//! - **the fractional slot**, which is not always representable —
//!   [`create_before`] carries the reposition path for an anchor with
//!   no room beneath it, and the rule that it may cost the *new* block
//!   a slot but may never move an existing sibling (issue #282);
//! - **the stale anchor**, because a client's selected-block id
//!   survives a peer reload, an external `.md` edit and a reconcile
//!   that re-minted the id. Every GUI routes "new block above / below
//!   the selection" through the `*_or_append` pair so the fallback
//!   cannot drift between them.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::{next_sibling, position_after, position_before};

use super::create::{append_block, create_with_position};
use super::{ensure_in_tree, wrap};

/// Insert a new sibling immediately after `after`, sharing the same
/// parent.
pub fn create_after(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    after: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    ensure_in_tree(workspace, after)?;
    let parent = workspace
        .tree()
        .parent(after)
        .ok_or_else(|| ActionError::NotInTree(after.to_string()))?;
    let position = position_after(workspace, after)
        .ok_or_else(|| ActionError::MissingPosition(after.to_string()))?;
    create_with_position(workspace, hlc, parent, position, text)
}

/// Insert a sibling after `after`, falling back to appending at the end
/// of `page` when `after` is no longer in the tree.
///
/// The frontend's selected-block id can go stale — a peer reload, an
/// external `.md` edit, or a reconcile that re-minted the block's id
/// leaves `after` pointing at a node no longer in the tree. Rather than
/// surfacing `block <id> is not in the tree` when the user just hit `o`
/// (or Enter for a new block), this appends the new block at the end of
/// `page` so the keystroke still produces a block.
///
/// Every GUI client (desktop, mobile) routes "new block after the
/// selection" through here so the stale-anchor fallback can't drift
/// between them — it used to be duplicated inline in each `create_block`
/// Tauri command.
pub fn create_after_or_append(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    page: NodeId,
    after: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    match create_after(workspace, hlc, after, text) {
        Err(ActionError::NotInTree(_)) => append_block(workspace, hlc, Some(page), text),
        other => other,
    }
}

/// Insert a new sibling immediately before `before`, sharing the same
/// parent.
///
/// Mirror of [`create_after`] for the "open a block above this one"
/// gesture (vim `O`, the desktop's `Cmd/Ctrl+Shift+Enter` with the
/// caret at column 0). The new block lands between `before` and its
/// preceding sibling, so the fractional index is computed by
/// [`position_before`].
///
/// **One case cannot be honoured, and it is a property of the index,
/// not a bug**: when the gap on *both* sides of `before` is empty there
/// is no key to mint and no slot to shift into, so the new block shares
/// `before`'s key and `tree::sort_siblings`' `NodeId` tiebreak decides
/// which of the two comes first. What the fallback below guarantees in
/// exchange is that no existing sibling moves. See the comment on that
/// branch.
pub fn create_before(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    before: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    ensure_in_tree(workspace, before)?;
    let parent = workspace
        .tree()
        .parent(before)
        .ok_or_else(|| ActionError::NotInTree(before.to_string()))?;

    if let Some(position) = position_before(workspace, before) {
        return create_with_position(workspace, hlc, parent, position, text);
    }

    // No representable slot beneath `before`. [`position_before`] names
    // the three shapes that get here: `before` at the fractional floor
    // (`Fractional::first()`), `before` tied with its predecessor, or a
    // predecessor exactly one `a` below it. Mirror what `move_up` does:
    // shift `before` up into the gap toward its next sibling, then drop
    // the new block into the freed slot so it lands ahead of `before`
    // while every sibling keeps its relative order.
    let floor = workspace
        .tree()
        .position(before)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(before.to_string()))?;
    let next_pos =
        next_sibling(workspace, before).and_then(|n| workspace.tree().position(n).cloned());
    let shifted = Fractional::between(Some(&floor), next_pos.as_ref());

    // ...but the gap *above* `before` can be empty too, and then there
    // is no slot to shift into either. `Fractional::between` drops an
    // upper bound it cannot satisfy, so `shifted` comes back sorting
    // PAST the next sibling, and the move would send `before` below a
    // block the user never touched. Two shapes reach it: `before` tied
    // with its successor (three offline creates under one parent), and
    // a successor that is `floor` plus a single `a` — `"aa"` is an
    // ordinary key, `between("a", "ab")` mints it, and nothing sorts
    // between `"a"` and `"aa"`.
    //
    // **A tie is not separable without choosing a `NodeId`**, and this
    // function does not get to choose one: the new block's id is minted
    // fresh, and `tree::sort_siblings` orders equal positions by id. So
    // the new block is allowed to land one slot off — it shares `floor`
    // with `before` and the id tiebreak decides which comes first — and
    // the existing siblings are not allowed to move at all. Relocating
    // content the user did not touch is the worse of the two, and it is
    // the one that looks like data loss.
    if next_pos.as_ref().is_none_or(|next| &shifted < next) {
        workspace.apply(wrap(
            hlc,
            Op::Move {
                node: before,
                new_parent: parent,
                position: shifted,
                old_parent: parent,
                old_position: floor.clone(),
            },
        ))?;
    }
    create_with_position(workspace, hlc, parent, floor, text)
}

/// Insert a sibling before `before`, falling back to appending at the end
/// of `page` when `before` is no longer in the tree.
///
/// The `create_before` counterpart of [`create_after_or_append`], for the
/// "open a block above this one" gesture (vim `O`, `Cmd/Ctrl+Shift+Enter` at
/// column 0). Same stale-anchor reality: a peer reload / re-mint can leave
/// `before` pointing at a node no longer in the tree, and appending the new
/// block at the page end beats surfacing `block <id> is not in the tree` when
/// the user just hit `O`. Every GUI client routes "new block above" through
/// here so the fallback can't drift.
pub fn create_before_or_append(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    page: NodeId,
    before: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    match create_before(workspace, hlc, before, text) {
        Err(ActionError::NotInTree(_)) => append_block(workspace, hlc, Some(page), text),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;

    fn new_workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    /// Regression: when the anchor has children, `create_after` must
    /// return the id of the brand-new sibling — not a descendant.
    ///
    /// Why: clients used to skip this return value and recover the new
    /// id by walking the refreshed outline (`flat[idx + 1]` after the
    /// anchor). That walk lands on `anchor.children[0]` instead of the
    /// new sibling whenever the anchor has expanded children, and the
    /// next `edit_text` would target a stale id and surface
    /// `block <ULID> is not in the tree` toasts on blur. The fix is
    /// to make every Tauri `create_block` command propagate
    /// `create_after`'s `NodeId` to the frontend; this test pins the
    /// contract on the `outl-actions` side so the regression cannot
    /// silently reappear.
    #[test]
    fn create_after_returns_new_sibling_not_a_child_of_anchor() {
        let (mut ws, hlc) = new_workspace();
        let anchor = append_block(&mut ws, &hlc, None, Some("anchor")).unwrap();
        let child = append_block(&mut ws, &hlc, Some(anchor), Some("child")).unwrap();

        let new_id = create_after(&mut ws, &hlc, anchor, Some("sibling")).unwrap();

        assert_ne!(new_id, child, "must not return the existing child");
        assert_eq!(
            ws.tree().parent(new_id),
            ws.tree().parent(anchor),
            "new block must be a sibling of the anchor (same parent)"
        );
        assert_eq!(ws.block_text(new_id).as_deref(), Some("sibling"));
    }

    #[test]
    fn create_after_or_append_inserts_sibling_when_anchor_is_live() {
        use crate::page::{open_or_create, PageKind};
        let (mut ws, hlc) = new_workspace();
        let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).unwrap();
        let a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
        let b = create_after_or_append(&mut ws, &hlc, page, a, Some("b")).unwrap();
        // Live anchor → ordinary sibling right after `a`.
        let kids: Vec<String> = crate::tree::children_of(&ws, page)
            .into_iter()
            .map(|(id, _)| ws.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(kids, vec!["a", "b"]);
        assert_eq!(ws.block_text(b).as_deref(), Some("b"));
    }

    #[test]
    fn create_after_or_append_falls_back_to_page_end_when_anchor_missing() {
        use crate::page::{open_or_create, PageKind};
        let (mut ws, hlc) = new_workspace();
        let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).unwrap();
        let _a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
        // A stale/re-minted id the frontend still holds: never created, so
        // it is NOT in the tree (the `o`-after-reconcile crash source). The
        // fallback must append at the page end, never error.
        let ghost = NodeId::new();
        let created = create_after_or_append(&mut ws, &hlc, page, ghost, Some("new")).unwrap();
        let kids: Vec<String> = crate::tree::children_of(&ws, page)
            .into_iter()
            .map(|(id, _)| ws.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(kids, vec!["a", "new"]);
        assert_eq!(ws.block_text(created).as_deref(), Some("new"));
    }

    #[test]
    fn create_before_or_append_inserts_sibling_when_anchor_is_live() {
        use crate::page::{open_or_create, PageKind};
        let (mut ws, hlc) = new_workspace();
        let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).unwrap();
        let a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
        // Live anchor → ordinary sibling right before `a`.
        let b = create_before_or_append(&mut ws, &hlc, page, a, Some("b")).unwrap();
        let kids: Vec<String> = crate::tree::children_of(&ws, page)
            .into_iter()
            .map(|(id, _)| ws.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(kids, vec!["b", "a"]);
        assert_eq!(ws.block_text(b).as_deref(), Some("b"));
    }

    #[test]
    fn create_before_or_append_falls_back_to_page_end_when_anchor_missing() {
        use crate::page::{open_or_create, PageKind};
        let (mut ws, hlc) = new_workspace();
        let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).unwrap();
        let _a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
        // A stale/re-minted id (a concurrent sync reload dropped the block the
        // frontend still points at): the `O`/new-block-above counterpart of the
        // `o` crash. The fallback must append at the page end, never error.
        let ghost = NodeId::new();
        let created = create_before_or_append(&mut ws, &hlc, page, ghost, Some("new")).unwrap();
        let kids: Vec<String> = crate::tree::children_of(&ws, page)
            .into_iter()
            .map(|(id, _)| ws.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(kids, vec!["a", "new"]);
        assert_eq!(ws.block_text(created).as_deref(), Some("new"));
    }

    #[test]
    fn create_before_inserts_sibling_directly_ahead_of_anchor() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();

        let new_id = create_before(&mut ws, &hlc, b, Some("between")).unwrap();

        assert_eq!(
            ws.tree().parent(new_id),
            ws.tree().parent(b),
            "new block must be a sibling of the anchor (same parent)"
        );
        let order: Vec<_> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![a, new_id, b], "new block lands between a and b");
        assert_eq!(ws.block_text(new_id).as_deref(), Some("between"));
    }

    #[test]
    fn create_before_first_child_lands_at_the_front() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();

        let new_id = create_before(&mut ws, &hlc, a, Some("head")).unwrap();

        let order: Vec<_> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            order,
            vec![new_id, a],
            "new block becomes the first sibling"
        );
    }

    #[test]
    fn create_before_first_of_many_reorders_via_floor_shift() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        let c = append_block(&mut ws, &hlc, None, Some("c")).unwrap();

        // `a` sits at the fractional floor; inserting before it must
        // shift `a` up and keep b / c in order.
        let new_id = create_before(&mut ws, &hlc, a, Some("head")).unwrap();

        let order: Vec<_> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![new_id, a, b, c]);
    }
}
