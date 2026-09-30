//! A parent's children, in the one order every device agrees on.
//!
//! [`sort_siblings`] is the single owner of that order, and its
//! `NodeId` tiebreak is load-bearing: two devices that both appended
//! while offline mint the same [`Fractional`], and
//! [`outl_core::tree::Tree::iter_nodes`] walks a process-seeded
//! `HashMap`, so ordering by position alone renders one op log two ways
//! (root `CLAUDE.md` invariant 7).
//!
//! Two ways to ask, and picking the wrong one is a real cost rather
//! than a style question: [`children_of`] is one full tree scan, right
//! for a single lookup and `O(nodes²)` for a walk; [`children_index`]
//! pays one scan per level of depth instead, for callers that visit a
//! whole subtree.

use std::collections::HashMap;

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

/// Put one parent's children into the canonical sibling order.
///
/// **The `NodeId` tiebreak is load-bearing, not tidiness.** Two devices
/// that both appended while offline produce two siblings holding the
/// same `Fractional`, and position alone is then not a total order.
/// [`outl_core::tree::Tree::iter_nodes`] walks a `HashMap` whose
/// iteration order is seeded per process, so a stable sort over a
/// partial comparator renders the same op log differently on every boot
/// — and differently on two devices, which is the convergence the op
/// log exists to provide (root `CLAUDE.md` invariant 7). Same reasoning
/// as the HLC's actor tiebreak: a comparison used to order replicated
/// state has to be total.
///
/// The single owner of that order. [`children_of`] and
/// `children_index` both call it, so the one-parent lookup and the
/// whole-subtree walk cannot disagree about where a tied sibling goes.
pub(crate) fn sort_siblings(rows: &mut [(NodeId, Fractional)]) {
    rows.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
}

/// Children of `parent` sorted ascending by their fractional position.
///
/// One full [`outl_core::tree::Tree::iter_nodes`] scan per call, because
/// `Tree` exposes no children accessor. That is the right cost for a
/// **single** lookup and the wrong one for a walk: asking it once per
/// visited node is `O(nodes²)`. Whole-subtree callers build a
/// `children_index` once instead.
pub fn children_of(workspace: &Workspace, parent: NodeId) -> Vec<(NodeId, Fractional)> {
    let mut rows: Vec<(NodeId, Fractional)> = workspace
        .tree()
        .iter_nodes()
        .filter(|(_, p, _)| *p == parent)
        .map(|(id, _, pos)| (id, pos.clone()))
        .collect();
    sort_siblings(&mut rows);
    rows
}

/// A pre-built `parent -> children` map, each entry in the canonical
/// sibling order `sort_siblings` defines.
///
/// Built once and reused across a whole traversal so recursive walking
/// / projection doesn't pay [`children_of`]'s per-call scan. A parent
/// with no children has **no entry** — `get` returning `None` and
/// returning an empty slice mean the same thing to every reader here.
pub type ChildrenIndex = HashMap<NodeId, Vec<NodeId>>;

/// `parent -> children` for `root`'s subtree, in the same order
/// [`children_of`] would return level by level.
///
/// **Scoped on purpose.** The obvious version of this fix groups the
/// *whole* workspace in one scan, which is right for a full-workspace
/// pass and wrong for the per-page calls that dominate: a page holds
/// tens of nodes, so a map over all 64k costs more than the walk it
/// replaces. This pays one scan per **level of depth** instead of one
/// per **node**, which is the win on both — outlines are far wider than
/// they are deep, and a subtree one node deep costs exactly what
/// `children_of` already cost.
///
/// Nodes outside `root`'s subtree are never in the map, so a walk of a
/// page cannot wander into the trash (deletion is
/// `Move(node, TRASH_ROOT)`, invariant 6, and trashed nodes stay in
/// `iter_nodes`). Pass [`NodeId::trash`] to index the deleted side.
pub(crate) fn children_index(workspace: &Workspace, root: NodeId) -> ChildrenIndex {
    let mut index = ChildrenIndex::new();
    let mut frontier: Vec<NodeId> = vec![root];
    // The materialised tree is acyclic (`Tree::creates_cycle` is what
    // buys that), so this only guards a corrupt one — without it a cycle
    // would loop here forever where the old recursion blew the stack.
    let budget = workspace.tree().node_count();
    let mut placed = 0usize;

    while !frontier.is_empty() {
        // The frontier stays **sorted** so membership is a binary
        // search rather than a hash. It is read once per node in the
        // whole tree, per level, so its constant is most of this
        // function's cost — and a page's frontier is a handful of ids,
        // where three comparisons beat hashing sixteen bytes. The
        // single-parent case (every walk's first level, and every level
        // of a narrow page) degenerates to one integer compare, which
        // is exactly what `children_of` pays.
        let single = (frontier.len() == 1).then(|| frontier[0]);
        let mut level: HashMap<NodeId, Vec<(NodeId, Fractional)>> = HashMap::new();
        for (id, parent, pos) in workspace.tree().iter_nodes() {
            let wanted = match single {
                Some(only) => parent == only,
                None => frontier.binary_search(&parent).is_ok(),
            };
            if wanted {
                level.entry(parent).or_default().push((id, pos.clone()));
            }
        }

        let mut next: Vec<NodeId> = Vec::with_capacity(level.values().map(Vec::len).sum());
        for (parent, mut kids) in level {
            sort_siblings(&mut kids);
            placed += kids.len();
            let ids: Vec<NodeId> = kids.into_iter().map(|(id, _)| id).collect();
            next.extend_from_slice(&ids);
            index.insert(parent, ids);
        }

        if placed > budget {
            break;
        }
        next.sort_unstable();
        frontier = next;
    }
    index
}

/// `parent -> children` for the **whole** tree — live nodes and the
/// trash alike — with siblings left in `iter_nodes` order.
///
/// The unsorted twin of `children_index`, and an explicit opt-out
/// rather than a second implementation: a caller reaches for it only
/// when it is collecting a *set* of nodes and something downstream
/// imposes the order (`crate::timeline` sorts its events by `Hlc`).
/// Anything that renders, projects or writes `.md` must use
/// [`children_index`] — sibling order there is convergence-critical,
/// and `HashMap` iteration is seeded per process.
pub(crate) fn children_index_unordered(workspace: &Workspace) -> ChildrenIndex {
    let mut map: ChildrenIndex = HashMap::new();
    for (id, parent, _) in workspace.tree().iter_nodes() {
        map.entry(parent).or_default().push(id);
    }
    map
}

/// `root` and everything under it, given a pre-built index.
///
/// The walk `timeline` needs to collect a deleted subtree and `trash`
/// needs to count one. It was written twice, in the same crate, within
/// fifty lines of each other's callers — the order they return differs
/// from [`children_of`]'s only in that it is a DFS, which neither
/// caller depends on.
///
/// A node missing from `index` contributes only itself, which is what
/// [`ChildrenIndex`]'s "no entry means no children" contract says.
pub(crate) fn subtree_ids(index: &ChildrenIndex, root: NodeId) -> Vec<NodeId> {
    let mut out = vec![root];
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        for &child in index.get(&node).into_iter().flatten() {
            out.push(child);
            stack.push(child);
        }
    }
    out
}

/// Previous sibling of `node` in its parent's children order.
pub(crate) fn previous_sibling(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let parent = workspace.tree().parent(node)?;
    let mut prev: Option<NodeId> = None;
    for (id, _) in children_of(workspace, parent) {
        if id == node {
            return prev;
        }
        prev = Some(id);
    }
    None
}

/// Next sibling of `node` in its parent's children order.
pub fn next_sibling(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let parent = workspace.tree().parent(node)?;
    let mut iter = children_of(workspace, parent).into_iter();
    while let Some((id, _)) = iter.next() {
        if id == node {
            return iter.next().map(|(id, _)| id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::op::{LogOp, Op};

    /// Two devices that both appended "first" while offline produce two
    /// siblings holding the **same** `Fractional`. Ordering them by
    /// position alone is not a total order, and
    /// `outl_core::Tree::iter_nodes` walks a `HashMap` whose iteration
    /// order is seeded per process — so the render of that page came out
    /// in a different order on every boot.
    ///
    /// That is a convergence bug before it is a churn bug: the same op
    /// log rendered two ways on two devices. It surfaced as churn
    /// because `reproject_stale_pages` rewrote the same journals on
    /// every pass of `outl serve` (measured on a real 2,860-page
    /// workspace: 1–3 pages rewritten per pass, forever).
    #[test]
    fn siblings_sharing_a_position_are_ordered_by_node_id() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let parent = NodeId::root();
        let tied = Fractional::first();

        let mut ids: Vec<NodeId> = (0..16).map(|_| NodeId::new()).collect();
        for id in &ids {
            let ts = hlc.next();
            ws.apply(LogOp {
                ts,
                actor: ts.actor,
                op: Op::Create {
                    node: *id,
                    parent,
                    position: tied.clone(),
                },
            })
            .unwrap();
        }

        let seen: Vec<NodeId> = children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        ids.sort();
        assert_eq!(
            seen, ids,
            "siblings at one position must have a total order, or the same op log \
         renders differently on two devices"
        );
    }
}
