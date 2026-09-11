//! Read-only navigation helpers over the materialised tree.

use std::collections::HashMap;

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::page::{KIND_KEY, SLUG_KEY};

/// Whether `key` is page-model book-keeping rather than a user
/// property.
///
/// `page-slug` / `page-kind` are written by [`crate::page`] through its
/// own ops; surfacing them in a rendered `.md` would rewrite the slug on
/// every reconcile, and surfacing them in the index would show the user
/// a property they never typed.
pub fn is_page_model_key(key: &str) -> bool {
    key == SLUG_KEY || key == KIND_KEY
}

/// A property value as the `.md` dialect renders it, or `None` when it
/// has no render syntax.
///
/// `Text`, `PageRef` and `Tag` all render as their string form, and the
/// parser reads `key:: [[x]]` / `key:: #x` back into the same shapes, so
/// the round trip closes. `List` has none yet and is dropped.
///
/// The single owner of that mapping. It exists because three callers
/// need it — the page renderer, the block renderer, and the tree-derived
/// index — and a block's properties must read the same whether they
/// arrive through the renderer or through the index.
///
/// Note this is **not** the rule [`text_properties_of`] applies: that
/// one keeps `Text` only. The two have disagreed since before either was
/// documented as an owner; whether the clipboard should be dropping
/// `PageRef` / `Tag` is a separate question from this one.
pub(crate) fn renderable_prop_value(value: &PropValue) -> Option<String> {
    match value {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => Some(s.clone()),
        PropValue::List(_) => None,
    }
}

/// Textual properties of `node`, minus the internal `page-slug` /
/// `page-kind` book-keeping keys, alphabetically sorted by key so the
/// output is stable across runs.
///
/// Only `PropValue::Text` survives — `PageRef` / `Tag` / `List` shapes
/// have no `.md` render syntax and are dropped silently. The single
/// owner of the "which block properties round-trip through the `.md`
/// dialect" rule; `clipboard::build_node` and any other serializer share
/// it instead of re-deriving the filter + sort.
pub(crate) fn text_properties_of(workspace: &Workspace, node: NodeId) -> Vec<(String, String)> {
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(node)
        .filter(|(k, _)| !is_page_model_key(k))
        .filter_map(|(k, v)| match v {
            PropValue::Text(s) => Some((k.to_string(), s.clone())),
            _ => None,
        })
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    properties
}

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

/// A fractional position strictly between `node` and the sibling that
/// follows it. Returns `None` when `node` is not in the tree.
///
/// Promoted to `pub` for the CLI's `outl block move --after=…` flow;
/// other clients can use it whenever they need a position immediately
/// after a known node.
pub fn position_after(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
    let parent = workspace.tree().parent(node)?;
    let siblings = children_of(workspace, parent);
    let mut iter = siblings.into_iter().peekable();
    while let Some((id, _)) = iter.next() {
        if id == node {
            let left = workspace.tree().position(node)?.clone();
            let right = iter.peek().map(|(_, p)| p.clone());
            return Some(Fractional::between(Some(&left), right.as_ref()));
        }
    }
    None
}

/// A fractional position strictly between `node` and the sibling that
/// precedes it, for the "insert before this node" flow (vim `O`,
/// `Cmd/Ctrl+Shift+Enter` with the caret at column 0 on the desktop).
///
/// Returns `None` when no such slot is representable — either `node`
/// is not in the tree, or it is the first child sitting at the
/// fractional floor (`Fractional::first()`), below which the `[a-z]`
/// index has no room. Callers handle the floor case by repositioning
/// (see [`crate::block::create_before`]), exactly how `move_up` swaps
/// rather than minting a sub-floor key.
pub fn position_before(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
    let node_pos = workspace.tree().position(node)?.clone();
    match previous_sibling(workspace, node) {
        Some(prev) => {
            let prev_pos = workspace.tree().position(prev)?.clone();
            Some(Fractional::between(Some(&prev_pos), Some(&node_pos)))
        }
        None => {
            // No predecessor: we need a slot below `node`. `between`
            // floors at `Fractional::first()`, so the result is only
            // valid when it actually sorts before `node`.
            let candidate = Fractional::between(None, Some(&node_pos));
            (candidate < node_pos).then_some(candidate)
        }
    }
}

/// Fractional position for a new last child appended under `parent`.
///
/// Promoted to `pub` for the CLI's `outl block move --parent=…` flow.
pub fn position_for_new_last_child(workspace: &Workspace, parent: NodeId) -> Fractional {
    let last = children_of(workspace, parent)
        .into_iter()
        .last()
        .map(|(_, p)| p);
    match last {
        Some(p) => Fractional::between(Some(&p), None),
        None => Fractional::first(),
    }
}

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
/// than a [`children_of`] per visited node. The closure still sees a
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
    let mut current = node;
    loop {
        if current == NodeId::trash() {
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

#[cfg(test)]
mod tests {
    use super::children_of;
    use outl_core::fractional::Fractional;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::op::LogOp;
    use outl_core::op::Op;
    use outl_core::workspace::Workspace;

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
