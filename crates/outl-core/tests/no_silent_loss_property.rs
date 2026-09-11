//! Invariant 5 (**no silent loss**) and the cycle no-op semantics of root
//! `CLAUDE.md` invariant 4 — *"a move that creates a cycle is a no-op on the
//! materialized tree, but the op still goes into the log"*.
//!
//! ## Why "the log has the same length" is not the assertion
//!
//! Every existing convergence property compares `OpLog::len()`. Length is a
//! weak witness: a reorder that dropped one op and duplicated another has the
//! right length and the wrong log, and so does any reorder that re-derives an
//! op's `old_*` fields into a *different op*. The assertions here are on the
//! set of HLC timestamps, read back through `OpLog::contains_ts` /
//! `OpLog::iter`, so "the log holds exactly the ops that were applied" is
//! checked as stated rather than counted.
//!
//! That distinction is the whole reason invariant 4 exists. An op the cycle
//! check turned into a tree no-op is invisible in the materialized state —
//! the log is the *only* place it survives, and it has to survive, because a
//! later-arriving op with a smaller HLC can reorder it into a move that is
//! legal after all. Deleting it "because it did nothing" is the single most
//! tempting wrong optimization in this crate, and it is silent: nothing
//! breaks until two devices disagree.
//!
//! ## Transitivity
//!
//! `creates_cycle` walks the whole ancestor chain, not just the immediate
//! parent. [`a_transitive_cycle_move_is_a_noop_but_stays_in_the_log`] builds
//! chains of depth 2 through 6 so a guard that only compared
//! `new_parent == node` or `parent(new_parent) == node` fails at depth 3 and
//! deeper. `cycle_chain.rs` pins one fixed depth; this sweeps them.
//!
//! ## Idempotency, per variant
//!
//! `idempotency.rs` covers `Create`, `Move` and a mixed sequence.
//! `convergence_property.rs` duplicates a whole random program. Neither states
//! the property one `Op` variant at a time, which is what breaks first when a
//! new variant is added or an existing branch is rewritten. Both directions
//! are pinned here: the same `LogOp` re-delivered (dedup by timestamp) and the
//! same *mutation* re-issued under fresh timestamps (no dedup — the tree has
//! to absorb it).

mod convergence_gen;

use convergence_gen::{find_cycle, lower, permutation, program_strategy, snapshot, Pools, Replica};
use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use proptest::prelude::*;
use std::collections::BTreeSet;

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

fn move_op(node: NodeId, new_parent: NodeId, position: &str) -> Op {
    Op::Move {
        node,
        new_parent,
        position: pos(position),
        old_parent: NodeId::root(),
        old_position: Fractional::first(),
    }
}

// --------------------------------------------------------------------------
// Cycle no-op semantics
// --------------------------------------------------------------------------

/// A `Move` onto a node's own descendant is rejected at **every** depth, and
/// the rejected op is still in the log afterwards.
///
/// Depth 2 is the case a naive `parent(new_parent) == node` check catches.
/// Depth 3+ is the case it does not — which is why this sweeps rather than
/// picking one.
#[test]
fn a_transitive_cycle_move_is_a_noop_but_stays_in_the_log() {
    for depth in 2..=6usize {
        let actor = ActorId::new();
        let nodes: Vec<NodeId> = (0..depth).map(|_| NodeId::new()).collect();
        let mut r = Replica::new(actor);

        // root → n0 → n1 → … → n(depth-1)
        for (i, node) in nodes.iter().enumerate() {
            let parent = if i == 0 { NodeId::root() } else { nodes[i - 1] };
            r.apply(at(
                i as u64 + 1,
                actor,
                Op::Create {
                    node: *node,
                    parent,
                    position: pos("m"),
                },
            ));
        }

        // Move the chain head under the chain tail: a cycle at distance
        // `depth - 1`.
        let cycle = at(100, actor, move_op(nodes[0], nodes[depth - 1], "n"));
        let cycle_ts = cycle.ts;
        r.apply(cycle);

        assert_eq!(
            r.tree.parent(nodes[0]),
            Some(NodeId::root()),
            "a cycle at depth {depth} was applied to the tree"
        );
        for (i, node) in nodes.iter().enumerate().skip(1) {
            assert_eq!(
                r.tree.parent(*node),
                Some(nodes[i - 1]),
                "the chain was disturbed at depth {depth}"
            );
        }
        assert!(
            r.log.contains_ts(&cycle_ts),
            "the rejected move was dropped from the log at depth {depth} \
             (invariant 4: no-op on the tree, still in the log)"
        );
        assert_eq!(r.log.len(), depth + 1, "log length at depth {depth}");
        assert!(
            find_cycle(&r.tree).is_none(),
            "cycle materialized at depth {depth}"
        );
    }
}

/// The same transitive cycle built **concurrently**: one actor grows the
/// chain `a → b → c`, another moves `a` under `c` at the same time.
///
/// The rejection has to happen on every replica and in every delivery order,
/// and the rejected op has to be present in every replica's log — otherwise
/// two devices hold different logs and the next late op replays differently
/// on each.
#[test]
fn a_concurrent_transitive_cycle_is_rejected_everywhere_and_loses_no_op() {
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();

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
        at(
            3,
            actor_a,
            Op::Create {
                node: c,
                parent: NodeId::root(),
                position: pos("m"),
            },
        ),
        at(10, actor_a, move_op(b, a, "n")),
        at(11, actor_a, move_op(c, b, "n")),
        // Concurrent with the two above: would close a → b → c → a.
        at(12, actor_b, move_op(a, c, "n")),
    ];
    let all_ts: Vec<Hlc> = ops.iter().map(|o| o.ts).collect();

    let mut baseline = None;
    for seed in [1u64, 2, 3, 5, 8, 13, 21, 34, 55] {
        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(ActorId::new());
        for &i in &order {
            r.apply(ops[i].clone());
        }

        assert!(
            find_cycle(&r.tree).is_none(),
            "cycle materialized under delivery seed {seed}"
        );
        for ts in &all_ts {
            assert!(
                r.log.contains_ts(ts),
                "op {ts:?} vanished from the log under delivery seed {seed}"
            );
        }
        assert_eq!(r.log.len(), ops.len(), "log length (seed {seed})");

        let snap = snapshot(&r.tree);
        match &baseline {
            None => baseline = Some(snap),
            Some(first) => assert_eq!(&snap, first, "replicas diverged (seed {seed})"),
        }
    }
}

// --------------------------------------------------------------------------
// Per-variant idempotency
// --------------------------------------------------------------------------

/// `root → a`, `a → b`, `root → c`. Returns the replica and the three ids.
fn seeded(actor: ActorId) -> (Replica, [NodeId; 3]) {
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
            parent: NodeId::root(),
            position: pos("m"),
        },
    ));
    (r, [a, b, c])
}

/// Every `Op` variant, as one representative mutation against [`seeded`].
///
/// `fresh` is the node a `Create` targets — the caller passes a stable id so
/// repeating the op repeats the *same* create, which is the whole point.
fn variant_op(kind: u8, nodes: [NodeId; 3], fresh: NodeId) -> Op {
    let [a, b, c] = nodes;
    match kind {
        0 => Op::Create {
            node: fresh,
            parent: a,
            position: pos("n"),
        },
        1 => move_op(b, c, "n"),
        2 => move_op(b, NodeId::trash(), "n"),
        3 => Op::SetProp {
            node: a,
            key: "icon".to_string(),
            value: Some(PropValue::Text("book".to_string())),
            old_value: None,
        },
        4 => Op::SetCollapsed {
            node: a,
            value: true,
            old_value: false,
        },
        5 => Op::SnoozeRemind {
            node: a,
            until_ms: Some(1_700_000_000_000),
            old_until_ms: None,
        },
        _ => Op::Edit {
            node: a,
            // Tree-level `Edit` is a no-op by design (text lives in
            // `Workspace`), so any bytes exercise the log path. The
            // content-level claim is
            // `re_applying_an_edit_never_duplicates_the_block_text`.
            text_op: vec![0u8, 1, 2, 3],
        },
    }
}

const VARIANTS: u8 = 7;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// INVARIANT 3, direction one — re-**delivery**. The identical `LogOp`
    /// arriving 2–5 times (P2P replay, an iCloud round trip, a plugin pull)
    /// must leave both the tree and the log exactly as one delivery did.
    ///
    /// Compared against itself rather than against a second replica: `seeded`
    /// mints fresh ids per call, so two replicas are never structurally
    /// comparable. "Applying it again changes nothing" is the same claim
    /// without the id mismatch.
    #[test]
    fn applying_the_same_logop_n_times_is_indistinguishable_from_applying_it_once(
        kind in 0u8..VARIANTS,
        repeats in 2usize..6,
    ) {
        let actor = ActorId::new();
        let fresh = NodeId::new();

        let (mut r, nodes) = seeded(actor);
        let op = at(10, actor, variant_op(kind, nodes, fresh));

        r.apply(op.clone());
        let after_one = snapshot(&r.tree);
        let log_after_one = r.log.len();

        for delivery in 1..repeats {
            r.apply(op.clone());
            prop_assert_eq!(
                &snapshot(&r.tree),
                &after_one,
                "delivery {} of variant {} changed the tree",
                delivery + 1,
                kind
            );
            prop_assert_eq!(
                r.log.len(),
                log_after_one,
                "delivery {} of variant {} inflated the log (ts dedup failed)",
                delivery + 1,
                kind
            );
        }
    }

    /// INVARIANT 3, direction two — re-**issue**. The same mutation emitted
    /// again under a *fresh* timestamp is not deduplicated (it is a genuinely
    /// new op and must be logged), and it must still leave the materialized
    /// tree untouched, because every variant's effect is a set-to-a-value.
    ///
    /// This is the direction a parent→children index is most likely to break:
    /// a second `Move` to the parent a node already has must not append the
    /// node to that parent's child list twice.
    #[test]
    fn re_issuing_the_same_mutation_under_fresh_timestamps_leaves_the_tree_unchanged(
        kind in 0u8..VARIANTS,
        repeats in 2u64..6,
    ) {
        let actor = ActorId::new();
        let fresh = NodeId::new();
        let (mut r, nodes) = seeded(actor);

        r.apply(at(10, actor, variant_op(kind, nodes, fresh)));
        let after_first = snapshot(&r.tree);
        let log_after_first = r.log.len();

        for tick in 1..repeats {
            r.apply(at(10 + tick, actor, variant_op(kind, nodes, fresh)));
            prop_assert_eq!(
                &snapshot(&r.tree),
                &after_first,
                "re-issuing variant {} changed the tree on repeat {}",
                kind,
                tick
            );
        }
        prop_assert_eq!(
            r.log.len(),
            log_after_first + (repeats - 1) as usize,
            "a re-issued op was dropped instead of logged (variant {})",
            kind
        );
    }

    /// INVARIANT 5 — every op applied is still in the log, identified by its
    /// timestamp, in every delivery order.
    ///
    /// Asserted as a set equality rather than a length, and over the full op
    /// mix, which is what makes it catch a reorder that swapped two ops as
    /// well as one that dropped an op.
    #[test]
    fn every_applied_op_is_still_in_the_log_under_any_delivery_order(
        program in program_strategy(),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);
        let expected: BTreeSet<Hlc> = ops.iter().map(|o| o.ts).collect();

        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(ops[i].clone());
            }

            let held: BTreeSet<Hlc> = r.log.iter().map(|o| o.ts).collect();
            prop_assert_eq!(
                &held,
                &expected,
                "the log is not the op set that was applied (seed {})",
                seed
            );
            prop_assert_eq!(
                r.log.len(),
                expected.len(),
                "the log holds a duplicate timestamp (seed {})",
                seed
            );
        }
    }

    /// INVARIANT 5 under cycle pressure specifically: a Move-only program
    /// biased to collide, where a large fraction of the ops *are* tree
    /// no-ops. Every one of them is still addressable in the log by its
    /// timestamp.
    #[test]
    fn an_op_the_cycle_guard_rejected_is_still_addressable_in_the_log(
        moves in prop::collection::vec((0usize..4, 0usize..4), 4..20),
        seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let actor = pools.actors[0];
        let mut ops: Vec<LogOp> = Vec::new();
        for (i, node) in pools.nodes.iter().take(4).enumerate() {
            ops.push(at(
                i as u64,
                actor,
                Op::Create {
                    node: *node,
                    parent: NodeId::root(),
                    position: pos("m"),
                },
            ));
        }
        for (k, (from, to)) in moves.iter().enumerate() {
            let parent = if from == to { NodeId::root() } else { pools.nodes[*to] };
            ops.push(at(
                100 + k as u64,
                pools.actors[k % pools.actors.len()],
                move_op(pools.nodes[*from], parent, "n"),
            ));
        }

        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(ActorId::new());
        for &i in &order {
            r.apply(ops[i].clone());
        }

        prop_assert!(find_cycle(&r.tree).is_none(), "cycle materialized");
        for op in &ops {
            prop_assert!(
                r.log.contains_ts(&op.ts),
                "an op the cycle guard rejected was dropped from the log"
            );
        }
    }
}

// --------------------------------------------------------------------------
// Edit: the one variant whose effect lives outside `Tree`
// --------------------------------------------------------------------------

/// `Op::Edit` is a tree-level no-op on purpose — block text lives in a Yrs
/// `Doc` owned by `Workspace`. So the idempotency claim for `Edit` has to be
/// made where its effect is: re-delivering the same update must not append the
/// text twice.
#[test]
fn re_applying_an_edit_never_duplicates_the_block_text() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    let node = NodeId::new();

    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor,
        op: Op::Create {
            node,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    })
    .expect("create");

    let update = ws.build_text_replace_update(node, "hello world");
    let edit_ts = hlc.next();
    let edit = LogOp {
        ts: edit_ts,
        actor,
        op: Op::Edit {
            node,
            text_op: update,
        },
    };

    for _ in 0..4 {
        ws.apply(edit.clone()).expect("edit");
    }

    assert_eq!(
        ws.block_text(node).as_deref(),
        Some("hello world"),
        "a re-delivered edit was merged more than once"
    );
    assert_eq!(
        ws.log().len(),
        2,
        "a re-delivered edit was appended to the log more than once"
    );
    assert!(ws.log().contains_ts(&edit_ts));
}
