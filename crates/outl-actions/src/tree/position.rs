//! Minting the fractional slot a new node goes into.
//!
//! [`Fractional::between`] guarantees a key **above** its lower bound
//! and one below the upper bound only when the gap holds a key at all.
//! On an empty gap it drops the upper bound, which is why the functions
//! here differ in what they can promise:
//! [`position_after`] never needs the bound it might lose, and
//! [`position_before`] exists to honour exactly that bound — so it
//! returns `None` rather than a key that sorts past the anchor the
//! caller asked to insert above.
//!
//! An empty gap is ordinary state, not corruption: a tie between two
//! siblings is what two offline devices produce, it lives in the op log,
//! and it replays. This used to be an `assert!(left < right)` inside
//! `between`, so it killed the process on every boot (issue #282).

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use super::children::{children_of, next_sibling, previous_sibling};

/// A fractional position strictly between `node` and the sibling that
/// follows it. Returns `None` when `node` is not in the tree.
///
/// Unlike [`position_before`] this never returns `None` for a tie, and
/// does not need to: when `node` and its next sibling share a position
/// the gap has no interior, [`Fractional::between`] drops the upper
/// bound, and the key still sorts after `node` — which is what the
/// caller asked for. The new block lands after the tied sibling too,
/// one slot further than intended. `position_before` has no such luck:
/// the bound the tie makes unsatisfiable is the one it exists to
/// honour.
pub fn position_after(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
    let left = workspace.tree().position(node)?.clone();
    let right = next_sibling(workspace, node).and_then(|n| workspace.tree().position(n).cloned());
    Some(Fractional::between(Some(&left), right.as_ref()))
}

/// A fractional position strictly between `node` and the sibling that
/// precedes it, for the "insert before this node" flow (vim `O`,
/// `Cmd/Ctrl+Shift+Enter` with the caret at column 0 on the desktop).
///
/// Returns `None` when no such slot is representable. Three shapes reach
/// that, and only the first is an error:
///
/// - `node` is not in the tree;
/// - `node` is the first child sitting at the fractional floor
///   (`Fractional::first()`), below which the `[a-z]` index has no room;
/// - `node` and its predecessor hold the **same** position. That is
///   ordinary state, not corruption — `sort_siblings` carries a
///   `NodeId` tiebreak precisely because two devices creating the first
///   child of one parent offline mint the same key — and a tie has no
///   interior, so no key sorts between the two.
///
/// Callers open a slot by repositioning (see
/// [`crate::block::create_before`]), exactly how `move_up` swaps rather
/// than minting a sub-floor key.
///
/// **The `< node_pos` check is the whole point of this function.**
/// [`Fractional::between`] guarantees a key above its lower bound and
/// promises one below the upper bound only when the gap holds a key at
/// all; on an empty gap it drops the upper bound and returns a key that
/// sorts at or after `node`. Using that key would insert the new block
/// *after* the anchor the user asked to insert above. This used to be an
/// `assert!(left < right)` inside `between` instead, which killed the
/// process on a tie — on every boot, since the tie lives in the op log
/// and replays (issue #282).
pub fn position_before(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
    let node_pos = workspace.tree().position(node)?.clone();
    let prev_pos =
        previous_sibling(workspace, node).and_then(|prev| workspace.tree().position(prev).cloned());
    let candidate = Fractional::between(prev_pos.as_ref(), Some(&node_pos));
    (candidate < node_pos).then_some(candidate)
}

/// Fractional position for a new last child appended under `parent`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::op::{LogOp, Op};

    /// Put `node` under `parent` at `pos`, straight through the op log.
    ///
    /// Tests here need to *choose* the position (that is the whole
    /// subject), which `append_block` does not let them do.
    fn place(
        ws: &mut Workspace,
        hlc: &HlcGenerator,
        node: NodeId,
        parent: NodeId,
        pos: &Fractional,
    ) {
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: Op::Create {
                node,
                parent,
                position: pos.clone(),
            },
        })
        .unwrap();
    }

    /// A tie between two siblings used to abort the process, not
    /// misplace a block.
    ///
    /// The tie itself is ordinary: `Fractional::between(None, None)` is
    /// deterministic, so two devices that both create the first child of
    /// one parent while offline mint the **same** position. Both ops are
    /// valid, both replay, and
    /// `siblings_sharing_a_position_are_ordered_by_node_id` above is the
    /// test that says so. Putting the cursor on the second one and asking
    /// for a slot above it (vim `O`) then reached
    /// `Fractional::between(Some(p), Some(p))`, whose `assert!(l < r)`
    /// killed the process — and because the tie lives in the op log, it
    /// replayed on every boot (issue #282).
    #[test]
    fn position_before_a_tied_sibling_does_not_panic() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let parent = NodeId::root();
        let tied = Fractional::first();

        let mut ids = [NodeId::new(), NodeId::new()];
        ids.sort();
        for id in &ids {
            place(&mut ws, &hlc, *id, parent, &tied);
        }

        // `ids[1]` is the second sibling under `sort_siblings`, so its
        // predecessor is `ids[0]` and both sit at `tied`.
        assert_eq!(super::previous_sibling(&ws, ids[1]), Some(ids[0]));
        let slot = super::position_before(&ws, ids[1]);

        // The gap between two equal keys is empty, so there is no
        // representable slot. `position_before` says so instead of
        // panicking, and `create_before` repositions.
        assert_eq!(
            slot, None,
            "a tie has no slot between it; the caller repositions"
        );
    }

    /// The issue's reproduction, end to end: press `O` on the second of
    /// two tied siblings.
    ///
    /// `create_before` asked [`super::position_before`] for a slot, got
    /// the panic, and the process died. Now it gets `None` and takes the
    /// route it already had for the fractional floor: shift the anchor up
    /// into the gap above it, then drop the new block into the freed key.
    ///
    /// What the tie costs is placement *relative to the predecessor*, not
    /// correctness: the new block shares the freed key with `ids[0]`, so
    /// the `NodeId` tiebreak decides which of the two comes first. Both
    /// devices replay the same op log through the same total order, so
    /// they agree — which is the property that matters (root `CLAUDE.md`
    /// invariant 7). Landing *above the anchor* is guaranteed, and that
    /// is what the user asked for.
    #[test]
    fn create_before_on_a_tied_sibling_lands_above_the_anchor() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let parent = NodeId::root();
        let tied = Fractional::first();

        let mut ids = [NodeId::new(), NodeId::new()];
        ids.sort();
        for id in &ids {
            place(&mut ws, &hlc, *id, parent, &tied);
        }

        let fresh = crate::block::create_before(&mut ws, &hlc, ids[1], Some("new")).unwrap();

        let order: Vec<NodeId> = children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let at = |id: NodeId| {
            order
                .iter()
                .position(|&x| x == id)
                .expect("still a sibling")
        };
        assert_eq!(order.len(), 3, "nothing was dropped: {order:?}");
        assert!(
            at(fresh) < at(ids[1]),
            "the new block must sort above the anchor: {order:?}"
        );
        assert!(
            at(ids[0]) < at(ids[1]),
            "the two original siblings keep their relative order: {order:?}"
        );
        assert_eq!(ws.block_text(fresh).as_deref(), Some("new"));

        // Convergence: a peer replaying the same ops in the opposite
        // order renders the same sibling order. The tie is split by a
        // `Move` in the log, not by anything local.
        let mut peer = Workspace::open_in_memory(actor).unwrap();
        let mut ops: Vec<LogOp> = ws.log().iter().cloned().collect();
        ops.reverse();
        for op in ops {
            peer.apply(op).unwrap();
        }
        let peer_order: Vec<NodeId> = children_of(&peer, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, peer_order, "two replicas must render one order");
    }

    /// The guard the two-sibling test above cannot reach.
    ///
    /// With only two tied siblings the anchor has no successor, so
    /// `create_before`'s reposition path calls
    /// `Fractional::between(floor, None)` and there is no upper bound to
    /// violate. Add a third and the bound appears — and it is tied with
    /// `floor`, so `between` drops it and hands back a key **past** the
    /// next sibling. Moving the anchor there relocates a block the user
    /// never touched: press `O` on the middle of three and the middle
    /// one lands last.
    ///
    /// A tie is not separable without choosing a `NodeId`, and
    /// `create_before` does not get to choose one. So the new block is
    /// allowed to land one slot off, and the existing siblings are not
    /// allowed to move at all.
    #[test]
    fn create_before_never_moves_a_sibling_it_cannot_clear() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let parent = NodeId::root();
        let tied = Fractional::first();

        let mut ids = [NodeId::new(), NodeId::new(), NodeId::new()];
        ids.sort();
        for id in &ids {
            place(&mut ws, &hlc, *id, parent, &tied);
        }

        // `O` on the middle sibling.
        let fresh = crate::block::create_before(&mut ws, &hlc, ids[1], Some("new")).unwrap();

        for id in &ids {
            assert_eq!(
                ws.tree().position(*id),
                Some(&tied),
                "no existing sibling may be moved to make room that does not exist"
            );
        }
        let order: Vec<NodeId> = children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            order.len(),
            4,
            "nothing dropped, nothing duplicated: {order:?}"
        );
        let at = |id: NodeId| {
            order
                .iter()
                .position(|&x| x == id)
                .expect("still a sibling")
        };
        assert!(
            at(ids[0]) < at(ids[1]) && at(ids[1]) < at(ids[2]),
            "the three originals keep their relative order: {order:?}"
        );
        assert_eq!(ws.block_text(fresh).as_deref(), Some("new"));
    }

    /// Same guard, the other empty gap, and this one predates the tie
    /// work: `"aa"` is an ordinary key (`between("a", "ab")` mints it),
    /// and nothing sorts between `"a"` and `"aa"`.
    ///
    /// So a first child at the floor whose successor is `floor + "a"`
    /// has no room below it *and* no room above it. The old bisection
    /// returned `"aam"` here and the anchor jumped below its successor,
    /// with no panic and no tie involved.
    #[test]
    fn create_before_never_moves_a_sibling_over_a_single_a_suffix() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let parent = NodeId::root();
        let floor = Fractional::first();
        let just_above = Fractional::parse("aa").unwrap();

        let anchor = NodeId::new();
        let successor = NodeId::new();
        place(&mut ws, &hlc, anchor, parent, &floor);
        place(&mut ws, &hlc, successor, parent, &just_above);

        let fresh = crate::block::create_before(&mut ws, &hlc, anchor, Some("new")).unwrap();

        assert_eq!(
            ws.tree().position(anchor),
            Some(&floor),
            "the anchor may not be moved past its successor"
        );
        assert_eq!(
            ws.tree().position(successor),
            Some(&just_above),
            "the successor may not move either"
        );
        let order: Vec<NodeId> = children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order.len(), 3, "nothing dropped: {order:?}");
        assert_eq!(
            order.last(),
            Some(&successor),
            "the successor stays last: {order:?}"
        );
        assert_eq!(ws.block_text(fresh).as_deref(), Some("new"));
    }
}
