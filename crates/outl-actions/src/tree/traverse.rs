//! Walking the tree from a node: down into its subtree, up to its page.
//!
//! Both directions are here because both are "follow parent / child
//! edges until something is true", and both are asked by every client
//! after a mutation on a deep block — the descent to visit what changed,
//! the ascent to name the page that has to be re-projected.

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use super::children::{children_index, ChildrenIndex};

/// Walk `parent`'s subtree in DFS pre-order and invoke `f` on every
/// descendant `NodeId`. Stops early when `f` returns `false`.
///
/// Centralises the "walk all descendants" pattern that the CLI's
/// `search`/`tag`/`query` paths used to reimplement individually.
/// Callers that need access to text/properties call into
/// [`Workspace::block_text`] / [`outl_core::Workspace::tree`] inside
/// the closure.
///
/// Resolves the whole subtree through one `children_index` rather
/// than a [`super::children_of`] per visited node. The closure still sees a
/// plain DFS pre-order and still stops the walk by returning `false` —
/// the index is built before the first call, so an early stop pays for
/// levels it never visits. That is the trade: bounded by depth, where
/// the per-node version was bounded by the whole workspace.
pub fn walk_subtree<F>(workspace: &Workspace, parent: NodeId, mut f: F)
where
    F: FnMut(NodeId) -> bool,
{
    let index = children_index(workspace, parent);
    walk_inner(&index, parent, &mut f);
}

fn walk_inner<F>(index: &ChildrenIndex, parent: NodeId, f: &mut F) -> bool
where
    F: FnMut(NodeId) -> bool,
{
    let Some(kids) = index.get(&parent) else {
        return true;
    };
    for &id in kids {
        if !f(id) {
            return false;
        }
        if !walk_inner(index, id, f) {
            return false;
        }
    }
    true
}

/// Walk up from `node` until we find the page node hosting it — that's
/// the highest ancestor sitting directly under [`NodeId::root`].
///
/// Returns `None` if `node` itself is the root (or detached). Lives in
/// `tree` because every client (CLI, future TUI handler, mobile) needs
/// it to re-render the page after a mutation on a deep block.
pub fn enclosing_page_id(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let mut current = node;
    loop {
        let parent = workspace.tree().parent(current)?;
        if parent == NodeId::root() {
            return Some(current);
        }
        current = parent;
    }
}

/// Whether `node`'s ancestor chain reaches the trash root.
///
/// **Not `tree().parent(node) == Some(NodeId::trash())`.** Deleting a
/// block is one `Op::Move` on the subtree *root*, and `do_op` rewrites
/// only that node's parent — every block under it keeps pointing at the
/// trashed root. So the direct-parent test answers `false` for the whole
/// inside of a deleted subtree, which is the majority of what a user
/// deleted.
pub fn is_trashed(workspace: &Workspace, node: NodeId) -> bool {
    is_under(workspace, node, NodeId::trash())
}

/// Whether `node` is `ancestor` or sits anywhere beneath it.
///
/// The walk [`is_trashed`] always was, with the sentinel as a
/// parameter. The second caller is `trash::landing_for`, which has to
/// ask "is this folded origin inside the very subtree we are
/// restoring": the origin was outside it at deletion, and a later
/// `Move` can have put it inside since.
///
/// Reflexive: a node is under itself, because both callers are asking
/// "would landing here be inside that subtree", and landing on the node
/// itself is the degenerate case of yes.
pub(crate) fn is_under(workspace: &Workspace, node: NodeId, ancestor: NodeId) -> bool {
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

/// The slug of the page hosting `node`, or `None` when the node is
/// detached (or is itself the root).
///
/// [`enclosing_page_id`] plus the page's `page-slug` — the pair every
/// caller that wants to *name* a block's page was writing out by hand.
/// `None` also covers a trashed block, whose ancestor chain no longer
/// reaches a page; a caller that must tell "deleted" from "detached"
/// checks the trash itself.
pub fn page_slug_of(workspace: &Workspace, node: NodeId) -> Option<String> {
    let page = enclosing_page_id(workspace, node)?;
    crate::page::page_meta(workspace, page).map(|meta| meta.slug)
}
