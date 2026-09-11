//! Invariant 4 — **the materialized state is always a valid tree** — and the
//! delete semantics that ride on it (invariant 6: delete is
//! `Move(node, TRASH_ROOT)`, never a physical removal).
//!
//! `convergence_property.rs` already asserts that every delivery order
//! materializes the *same* tree. Same is not the same as *valid*: two replicas
//! agreeing on a malformed shape converge perfectly and are still broken. The
//! only shape assertion in the existing battery is
//! `convergence_gen::find_cycle`, and it is reached from exactly one property
//! (`concurrent_moves_never_cycle`), over a **Move-only** program. Nothing
//! checked the shape after the full op mix, and nothing at all checked that a
//! node's parent chain *terminates*.
//!
//! ## What "valid" means here, precisely
//!
//! Not "every node reaches ROOT or TRASH_ROOT". That claim is false by design:
//! a `Create` or `Move` whose parent has no node record yet materializes
//! against a **phantom parent** and stays that way until the parent's own op
//! arrives (`create_with_phantom_parent_materializes_then_resolves`). A test
//! asserting root-reachability would fail on every legitimate out-of-order
//! delivery, and the way to make it pass would be to stop laying down the
//! edge — i.e. to silently drop an op.
//!
//! The property that *is* true, and that a parent→children index or a reworked
//! `undo_op` could break, is weaker in wording and just as strong in effect:
//! **every node's parent chain terminates**, at ROOT, at TRASH_ROOT, or at a
//! phantom. Never at itself. [`ChainEnd`] names the three legal endings so a
//! failure says which one was expected.
//!
//! ## Deliberately public-API only
//!
//! Everything here goes through `Tree::parent` / `contains` / `iter_nodes` and
//! `OpLog::len`. A test that reached into `Tree::nodes` would pass by
//! construction against the very index refactor it is meant to protect: the
//! whole point of that refactor is a second representation of the same edges,
//! and a test reading one map cannot notice the other disagreeing.

mod convergence_gen;

use convergence_gen::{find_cycle, lower, permutation, program_strategy, snapshot, Pools, Replica};
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::tree::Tree;
use proptest::prelude::*;
use std::collections::BTreeSet;

// --------------------------------------------------------------------------
// Shape vocabulary
// --------------------------------------------------------------------------

/// Where a node's parent chain ends.
///
/// Three of these are legal; [`ChainEnd::Cycle`] is invariant 4 broken.
#[derive(Debug, PartialEq, Eq)]
enum ChainEnd {
    /// Reached `NodeId::root()` — a live node.
    Root,
    /// Reached `NodeId::trash()` — a deleted node or a descendant of one.
    Trash,
    /// Reached a parent with no node record. Legal and expected: the
    /// parent's own `Create` has not been delivered yet.
    Phantom(NodeId),
    /// The walk revisited a node it had already seen. Invariant 4 violated.
    Cycle(NodeId),
}

/// Walk `start`'s parent chain to its end.
///
/// Termination is guaranteed by `seen`, not by a step budget: either a
/// sentinel / phantom ends the walk or a repeat is detected, and both happen
/// within `node_count` steps. Deliberately independent of
/// `Tree::creates_cycle` — a test that asked the implementation whether it had
/// made a cycle would agree with it about a wrong answer.
fn chain_end(tree: &Tree, start: NodeId) -> ChainEnd {
    let mut seen: BTreeSet<NodeId> = BTreeSet::new();
    seen.insert(start);
    let mut cursor = start;
    loop {
        let parent = match tree.parent(cursor) {
            Some(p) => p,
            None => return ChainEnd::Phantom(cursor),
        };
        if parent == NodeId::root() {
            return ChainEnd::Root;
        }
        if parent == NodeId::trash() {
            return ChainEnd::Trash;
        }
        if !seen.insert(parent) {
            return ChainEnd::Cycle(parent);
        }
        cursor = parent;
    }
}

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid position")
}

fn at(physical_ms: u64, actor: ActorId, op: Op) -> LogOp {
    LogOp {
        ts: Hlc::new(physical_ms, 0, actor),
        actor,
        op,
    }
}

fn trash_op(node: NodeId, position: &str) -> Op {
    Op::Move {
        node,
        new_parent: NodeId::trash(),
        position: pos(position),
        old_parent: NodeId::root(),
        old_position: Fractional::first(),
    }
}

fn move_op(node: NodeId, new_parent: NodeId, position: &str) -> Op {
    Op::Move {
        node,
        new_parent,
        position: pos(position),
        old_parent: NodeId::root(),
        old_position: Fractional::first(),
    }
}

/// `root → a → b → c`, one actor, physical times 1..=3.
fn chain_of_three(actor: ActorId) -> (Replica, [NodeId; 3]) {
    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let mut r = Replica::new(actor);
    r.apply(at(
        1,
        actor,
        Op::Create {
            node: a,
            parent: NodeId::root(),
            position: pos("m"),
        },
    ));
    r.apply(at(
        2,
        actor,
        Op::Create {
            node: b,
            parent: a,
            position: pos("m"),
        },
    ));
    r.apply(at(
        3,
        actor,
        Op::Create {
            node: c,
            parent: b,
            position: pos("m"),
        },
    ));
    (r, [a, b, c])
}

// --------------------------------------------------------------------------
// Deterministic: delete is a Move, and a Move preserves the subtree
// --------------------------------------------------------------------------

/// Invariant 6, stated as the thing a reader can check: after a delete the
/// node is **still in the tree**, parented under `TRASH_ROOT`.
///
/// The failure mode this guards is an "optimization" that physically removes
/// the node — which loses the op's undo record and makes a later reorder
/// unable to bring it back.
#[test]
fn a_delete_leaves_the_node_in_the_tree_parented_under_the_trash_root() {
    let actor = ActorId::new();
    let (mut r, [a, b, _c]) = chain_of_three(actor);

    r.apply(at(4, actor, trash_op(b, "n")));

    assert!(
        r.tree.contains(b),
        "delete removed the node instead of moving it to trash"
    );
    assert_eq!(r.tree.parent(b), Some(NodeId::trash()));
    assert_eq!(chain_end(&r.tree, b), ChainEnd::Trash);
    assert_eq!(chain_end(&r.tree, a), ChainEnd::Root, "a was not deleted");
    assert_eq!(r.log.len(), 4, "the delete is an op like any other");
}

/// A trashed node's descendants are not re-parented — they keep pointing at
/// their parent and therefore resolve to the trash *through* it.
///
/// This is what makes "restore" a single op rather than a subtree walk, and a
/// parent→children index that eagerly detaches children on a move to trash
/// would fail here.
#[test]
fn a_trashed_subtree_keeps_every_descendant_beneath_the_trash_root() {
    let actor = ActorId::new();
    let (mut r, [_a, b, c]) = chain_of_three(actor);

    r.apply(at(4, actor, trash_op(b, "n")));

    assert_eq!(
        r.tree.parent(c),
        Some(b),
        "the descendant was re-parented by a delete it was not the target of"
    );
    assert_eq!(chain_end(&r.tree, c), ChainEnd::Trash);
}

/// Restore is the mirror image: one `Move` back out of the trash brings the
/// whole subtree with it, because the subtree never left.
#[test]
fn restoring_a_trashed_node_brings_its_whole_subtree_back_under_the_root() {
    let actor = ActorId::new();
    let (mut r, [a, b, c]) = chain_of_three(actor);

    r.apply(at(4, actor, trash_op(b, "n")));
    r.apply(at(5, actor, move_op(b, a, "o")));

    assert_eq!(chain_end(&r.tree, b), ChainEnd::Root);
    assert_eq!(chain_end(&r.tree, c), ChainEnd::Root);
    assert_eq!(
        r.tree.parent(c),
        Some(b),
        "the subtree survived the round trip"
    );
    assert_eq!(r.log.len(), 5);
}

/// Trash, then a *late* op that reorders around it: the delete still wins,
/// because it is later in the HLC order, and it wins in every delivery order.
///
/// The reorder path (`undo_op` → `do_op`) is where a Move to trash is most
/// likely to be mishandled, since `undo_op`'s `Move` branch only reverts when
/// the current parent still matches `new_parent`.
#[test]
fn a_delete_survives_an_earlier_op_arriving_after_it() {
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let a = NodeId::new();
    let b = NodeId::new();

    let ops = [
        at(
            1,
            actor_a,
            Op::Create {
                node: a,
                parent: NodeId::root(),
                position: pos("m"),
            },
        ),
        at(
            2,
            actor_a,
            Op::Create {
                node: b,
                parent: NodeId::root(),
                position: pos("m"),
            },
        ),
        // Concurrent: b moves under a at 10, and is trashed at 11.
        at(10, actor_b, move_op(b, a, "n")),
        at(11, actor_a, trash_op(b, "o")),
    ];

    for seed in [1u64, 2, 3, 5, 8, 13, 21, 34] {
        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(ActorId::new());
        for &i in &order {
            r.apply(ops[i].clone());
        }
        assert_eq!(
            r.tree.parent(b),
            Some(NodeId::trash()),
            "the later delete lost to an earlier move (seed {seed})"
        );
        assert_eq!(
            r.log.len(),
            4,
            "no op lost across the reorder (seed {seed})"
        );
    }
}

// --------------------------------------------------------------------------
// Properties
// --------------------------------------------------------------------------

proptest! {
    // 96 cases × up to 4 permutations each. Keeps the file comfortably under
    // a second in debug while still shrinking usefully on failure.
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// INVARIANT 4 — every node's parent chain terminates, in every delivery
    /// order, over the **full** op mix (`Create` / `Move` / delete /
    /// `SetProp` / `SetCollapsed` / `SnoozeRemind`).
    ///
    /// The existing suite only ever checked shape after a Move-only program.
    #[test]
    fn every_node_parent_chain_terminates_at_root_trash_or_a_phantom(
        program in program_strategy(),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(ops[i].clone());
            }
            let nodes: Vec<NodeId> = r.tree.iter_nodes().map(|(n, _, _)| n).collect();
            for node in nodes {
                let end = chain_end(&r.tree, node);
                prop_assert!(
                    !matches!(end, ChainEnd::Cycle(_)),
                    "node {node} sits on a cycle: {end:?}"
                );
            }
        }
    }

    /// Same invariant reached from the other side: `find_cycle` — the
    /// battery's own detector — over the full op mix rather than Moves alone.
    ///
    /// Two detectors for one property is not redundancy here. `find_cycle`
    /// walks a bounded number of steps from every node looking for the start;
    /// `chain_end` walks to termination with a visited set. A cycle that does
    /// not contain the node it was entered from is visible to the second and
    /// not to the first.
    #[test]
    fn no_node_is_its_own_ancestor_under_any_delivery_order(
        program in program_strategy(),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(ops[i].clone());
            }
            prop_assert!(
                find_cycle(&r.tree).is_none(),
                "a cycle materialized under delivery seed {}",
                seed
            );
        }
    }

    /// INVARIANT 6, as a property: trash every node in the pool with the
    /// highest timestamps in the program, and every node that exists must
    /// resolve **directly** to `TRASH_ROOT` — in any delivery order.
    ///
    /// "Directly" is the sharp part. Each node gets its own delete, so a
    /// correct tree has `parent(n) == trash()` for all of them; a tree that
    /// let one delete be swallowed (by a cycle guard misreading the trash
    /// sentinel, say) leaves that node pointing at a sibling instead.
    #[test]
    fn every_node_deleted_last_resolves_directly_to_the_trash_root(
        program in program_strategy(),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        let base = ops.len() as u64;
        let actor = pools.actors[0];
        for (i, node) in pools.nodes.iter().enumerate() {
            ops.push(at(base + i as u64, actor, trash_op(*node, "z")));
        }

        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(ops[i].clone());
            }
            for node in &pools.nodes {
                if r.tree.contains(*node) {
                    prop_assert_eq!(
                        r.tree.parent(*node),
                        Some(NodeId::trash()),
                        "a delete was swallowed under delivery seed {}",
                        seed
                    );
                }
            }
        }
    }

    /// A delete that is last by HLC wins regardless of when it is *delivered*,
    /// and every replica agrees on the whole tree afterwards.
    ///
    /// Deliberately pairs the shape assertion with a convergence assertion:
    /// "b is in the trash everywhere" and "the trees are identical" are
    /// different failures, and a reorder bug can produce either one alone.
    #[test]
    fn a_delete_that_is_last_by_hlc_wins_in_every_delivery_order(
        program in program_strategy(),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        let base = ops.len() as u64;
        let actor = pools.actors[0];
        let victim = pools.nodes[0];
        let rival = pools.nodes[1];

        // A competing move, then the delete one tick later.
        ops.push(at(base, actor, move_op(victim, rival, "y")));
        ops.push(at(base + 1, actor, trash_op(victim, "z")));

        let mut baseline: Option<_> = None;
        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(ops[i].clone());
            }
            if r.tree.contains(victim) {
                prop_assert_eq!(
                    r.tree.parent(victim),
                    Some(NodeId::trash()),
                    "the earlier move beat the later delete (seed {})",
                    seed
                );
            }
            let snap = snapshot(&r.tree);
            match &baseline {
                None => baseline = Some(snap),
                Some(first) => prop_assert_eq!(&snap, first, "replicas diverged (seed {})", seed),
            }
        }
    }
}
