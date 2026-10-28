//! Structural moves over the materialised tree.
//!
//! Everything in this module re-parents or reorders an *existing*
//! node — it never creates or edits block text (that lives in the
//! parent [`crate::block`] module). Each function reads the current
//! tree to compute the `Op::Move` parameters and routes the op
//! through [`Workspace::apply`] so the op log stays the single source
//! of truth.
//!
//! Per invariant #6, **delete is a move to the trash root**, not a
//! physical removal — [`delete`] is just `move_to(node, TRASH_ROOT)`.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::{next_sibling, position_after, position_for_new_last_child, previous_sibling};

use super::{ensure_in_tree, wrap};

fn move_to(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    new_parent: NodeId,
    position: Fractional,
) -> Result<(), ActionError> {
    let old_parent = workspace
        .tree()
        .parent(node)
        .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
    let old_position = workspace
        .tree()
        .position(node)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(node.to_string()))?;
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node,
            new_parent,
            position,
            old_parent,
            old_position,
        },
    ))?;
    Ok(())
}

/// Move the block under the trash root. Materialised tree drops it
/// immediately; the op stays in the log forever.
pub fn delete(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    move_to(workspace, hlc, node, NodeId::trash(), Fractional::first())
}

/// Move `node` to become the last child of `new_parent`.
///
/// The cross-page / arbitrary re-parent the plugin host needs (e.g. archive a
/// block under another page). A move that would create a cycle is a
/// deterministic no-op on the materialized tree, but the op still lands in the
/// log (root invariant #4) — that is handled inside `apply`, not here.
pub fn move_under(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    new_parent: NodeId,
) -> Result<(), ActionError> {
    // `ensure_in_tree`, not `ensure_live`: `trash::restore` is a
    // `move_under` out of the trash, so a node sitting there has to be
    // a legal source here. The destination is checked the same way
    // because `delete` aims one at `TRASH_ROOT` through `move_to`.
    ensure_in_tree(workspace, node)?;
    ensure_in_tree(workspace, new_parent)?;
    let pos = position_for_new_last_child(workspace, new_parent);
    move_to(workspace, hlc, node, new_parent, pos)
}

/// Indent `node` so it becomes the last child of its previous sibling.
pub fn indent(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let prev = previous_sibling(workspace, node)
        .ok_or_else(|| ActionError::NoPreviousSibling(node.to_string()))?;
    let pos = position_for_new_last_child(workspace, prev);
    move_to(workspace, hlc, node, prev, pos)
}

/// Outdent `node` so it becomes a sibling of its current parent.
pub fn outdent(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let parent = workspace
        .tree()
        .parent(node)
        .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
    if parent == NodeId::root() {
        return Err(ActionError::AlreadyAtRoot(node.to_string()));
    }
    if workspace
        .tree()
        .property(parent, crate::page::SLUG_KEY)
        .is_some()
    {
        return Err(ActionError::AlreadyAtRoot(node.to_string()));
    }
    let grand = workspace
        .tree()
        .parent(parent)
        .ok_or_else(|| ActionError::NoGrandparent(node.to_string()))?;
    let pos = position_after(workspace, parent)
        .ok_or_else(|| ActionError::MissingPosition(parent.to_string()))?;
    move_to(workspace, hlc, node, grand, pos)
}

/// Swap `node` with its previous sibling. No-op if `node` is already
/// the first sibling.
pub fn move_up(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let prev = match previous_sibling(workspace, node) {
        Some(p) => p,
        None => return Ok(()),
    };
    swap_positions(workspace, hlc, node, prev)
}

/// Swap `node` with its next sibling. No-op if `node` is already the
/// last sibling.
pub fn move_down(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let next = match next_sibling(workspace, node) {
        Some(n) => n,
        None => return Ok(()),
    };
    swap_positions(workspace, hlc, node, next)
}

/// Move `node` so it becomes the sibling immediately **after**
/// `target`, re-parenting it under `target`'s parent.
///
/// This is the workspace-level primitive behind the desktop's
/// cut-and-paste-block gesture (`Cmd+X` then `Cmd+V`): the node keeps
/// its identity — and therefore every `((blk-…))` ref and backlink
/// pointing at it stays valid — so a "paste" is a single [`Op::Move`],
/// never a delete + recreate. Because `target` can live on any page,
/// this also covers moving a block across pages.
///
/// Rejects a paste that would drop the node inside its own subtree
/// (pasting an ancestor onto one of its descendants) with
/// [`ActionError::WouldCreateCycle`]: the CRDT would silently no-op
/// such a move, so the client should nudge instead. Pasting a block
/// after itself is a no-op (returns `Ok`).
pub fn move_after(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    target: NodeId,
) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;
    ensure_in_tree(workspace, target)?;
    if node == target {
        return Ok(());
    }
    // `target` must not sit inside `node`'s own subtree — that move
    // creates a cycle the CRDT drops on the materialised tree.
    if is_within_subtree(workspace, node, target) {
        return Err(ActionError::WouldCreateCycle(node.to_string()));
    }
    let parent = workspace
        .tree()
        .parent(target)
        .ok_or_else(|| ActionError::NotInTree(target.to_string()))?;
    let position = position_after(workspace, target)
        .ok_or_else(|| ActionError::MissingPosition(target.to_string()))?;
    move_to(workspace, hlc, node, parent, position)
}

/// True when `ancestor` lies on the path from `node` up to the root —
/// i.e. `node` lives inside `ancestor`'s subtree (`node == ancestor`
/// counts). Walks parents rather than the subtree because the path to
/// root is at most tree-depth long, regardless of how wide the subtree
/// under `ancestor` is.
fn is_within_subtree(workspace: &Workspace, ancestor: NodeId, node: NodeId) -> bool {
    let mut current = node;
    loop {
        if current == ancestor {
            return true;
        }
        match workspace.tree().parent(current) {
            Some(parent) => current = parent,
            None => return false,
        }
    }
}

/// Swap the positions of two nodes by emitting two `Move` ops. They
/// keep the same parent (this is a sibling-swap helper); use
/// [`indent`] / [`outdent`] for re-parenting.
fn swap_positions(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    a: NodeId,
    b: NodeId,
) -> Result<(), ActionError> {
    let parent_a = workspace
        .tree()
        .parent(a)
        .ok_or_else(|| ActionError::NotInTree(a.to_string()))?;
    let parent_b = workspace
        .tree()
        .parent(b)
        .ok_or_else(|| ActionError::NotInTree(b.to_string()))?;
    let pos_a = workspace
        .tree()
        .position(a)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(a.to_string()))?;
    let pos_b = workspace
        .tree()
        .position(b)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(b.to_string()))?;

    // Move a → b's slot.
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node: a,
            new_parent: parent_b,
            position: pos_b.clone(),
            old_parent: parent_a,
            old_position: pos_a.clone(),
        },
    ))?;
    // Move b → a's old slot.
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node: b,
            new_parent: parent_a,
            position: pos_a,
            old_parent: parent_b,
            old_position: pos_b,
        },
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::tree::children_of;
    use outl_core::id::ActorId;

    fn new_workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn indent_makes_block_child_of_previous_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        indent(&mut ws, &hlc, b).unwrap();
        assert_eq!(ws.tree().parent(b), Some(a));
    }

    #[test]
    fn outdent_promotes_to_grandparent_level() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        indent(&mut ws, &hlc, b).unwrap();
        outdent(&mut ws, &hlc, b).unwrap();
        assert_eq!(ws.tree().parent(b), Some(NodeId::root()));
        // a stays where it is
        assert_eq!(ws.tree().parent(a), Some(NodeId::root()));
    }

    #[test]
    fn delete_moves_to_trash() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("trash me")).unwrap();
        delete(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.tree().parent(n), Some(NodeId::trash()));
    }

    #[test]
    fn indent_rejects_first_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        assert!(matches!(
            indent(&mut ws, &hlc, a),
            Err(ActionError::NoPreviousSibling(_))
        ));
    }

    #[test]
    fn outdent_rejects_root_level_block() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        assert!(matches!(
            outdent(&mut ws, &hlc, a),
            Err(ActionError::AlreadyAtRoot(_))
        ));
    }

    #[test]
    fn outdent_top_level_page_block_is_rejected_not_deleted() {
        use crate::page::{open_or_create, PageKind};
        let (mut ws, hlc) = new_workspace();
        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        let block = append_block(&mut ws, &hlc, Some(page), Some("a top-level block")).unwrap();

        assert!(matches!(
            outdent(&mut ws, &hlc, block),
            Err(ActionError::AlreadyAtRoot(_))
        ));
        // The block stays put under its page.
        assert_eq!(ws.tree().parent(block), Some(page));
    }

    #[test]
    fn outdent_nested_page_block_promotes_within_page() {
        use crate::page::{open_or_create, PageKind};
        let (mut ws, hlc) = new_workspace();
        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        let parent = append_block(&mut ws, &hlc, Some(page), Some("parent")).unwrap();
        let child = append_block(&mut ws, &hlc, Some(parent), Some("child")).unwrap();

        outdent(&mut ws, &hlc, child).unwrap();
        assert_eq!(ws.tree().parent(child), Some(page));
    }

    #[test]
    fn move_up_swaps_with_previous_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_up(&mut ws, &hlc, b).unwrap();
        let order: Vec<NodeId> = children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![b, a]);
    }

    #[test]
    fn move_down_swaps_with_next_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_down(&mut ws, &hlc, a).unwrap();
        let order: Vec<NodeId> = children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![b, a]);
    }

    #[test]
    fn move_up_first_sibling_is_noop() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_up(&mut ws, &hlc, a).unwrap();
        let order: Vec<NodeId> = children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![a, b]);
    }

    #[test]
    fn move_after_reorders_within_siblings() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        let c = append_block(&mut ws, &hlc, None, Some("c")).unwrap();
        // Move `a` to sit right after `c`: [a, b, c] → [b, c, a].
        move_after(&mut ws, &hlc, a, c).unwrap();
        let order: Vec<NodeId> = children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![b, c, a]);
    }

    #[test]
    fn move_after_reparents_across_subtrees() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let a1 = append_block(&mut ws, &hlc, Some(a), Some("a1")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        // Paste `b` after `a1` (a's child) → b becomes a's child too.
        move_after(&mut ws, &hlc, b, a1).unwrap();
        assert_eq!(ws.tree().parent(b), Some(a));
    }

    #[test]
    fn move_after_into_own_subtree_is_rejected() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let a1 = append_block(&mut ws, &hlc, Some(a), Some("a1")).unwrap();
        let log_before = ws.log().len();
        // Pasting `a` after its own descendant `a1` would create a cycle.
        assert!(matches!(
            move_after(&mut ws, &hlc, a, a1),
            Err(ActionError::WouldCreateCycle(_))
        ));
        // Tree is untouched.
        assert_eq!(ws.tree().parent(a), Some(NodeId::root()));
        assert_eq!(ws.tree().parent(a1), Some(a));
        // No op leaked into the log: the guard rejects before minting a
        // `LogOp`, so nothing reaches `Workspace::apply`.
        assert_eq!(ws.log().len(), log_before);
    }

    #[test]
    fn move_after_onto_self_is_noop() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_after(&mut ws, &hlc, a, a).unwrap();
        let order: Vec<NodeId> = children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![a, b]);
    }

    /// A cut → paste moves the whole subtree, at every depth, in one
    /// `Op::Move`. The CRDT re-parents the node by id, so descendants
    /// ride along untouched — we never walk the subtree to move it
    /// child-by-child, and nothing is left behind under the old parent.
    ///
    /// Tree before (under root):
    /// ```text
    /// - src
    ///   - c1
    ///     - c1a
    ///       - c1a_i
    ///     - c1b
    ///   - c2
    /// - dest
    /// ```
    /// After `move_after(src, dest)` the entire `src` subtree hangs off
    /// root after `dest`, with every parent/child edge preserved.
    #[test]
    fn move_after_carries_full_deep_subtree() {
        let (mut ws, hlc) = new_workspace();
        let src = append_block(&mut ws, &hlc, None, Some("src")).unwrap();
        let c1 = append_block(&mut ws, &hlc, Some(src), Some("c1")).unwrap();
        let c1a = append_block(&mut ws, &hlc, Some(c1), Some("c1a")).unwrap();
        let c1a_i = append_block(&mut ws, &hlc, Some(c1a), Some("c1a_i")).unwrap();
        let c1b = append_block(&mut ws, &hlc, Some(c1), Some("c1b")).unwrap();
        let c2 = append_block(&mut ws, &hlc, Some(src), Some("c2")).unwrap();
        let dest = append_block(&mut ws, &hlc, None, Some("dest")).unwrap();

        move_after(&mut ws, &hlc, src, dest).unwrap();

        // `src` re-parented to root, sitting right after `dest`.
        let roots: Vec<NodeId> = children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(roots, vec![dest, src]);

        // Every descendant edge survived the move at all four levels.
        assert_eq!(ws.tree().parent(c1), Some(src));
        assert_eq!(ws.tree().parent(c2), Some(src));
        assert_eq!(ws.tree().parent(c1a), Some(c1));
        assert_eq!(ws.tree().parent(c1b), Some(c1));
        assert_eq!(ws.tree().parent(c1a_i), Some(c1a));

        // Sibling order under each parent is unchanged.
        let src_kids: Vec<NodeId> = children_of(&ws, src)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(src_kids, vec![c1, c2]);
    }
}
