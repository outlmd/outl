//! `do_op(Op::Create)` is idempotent; `undo_op` must be its exact inverse
//! **including for the applications that did nothing**.
//!
//! ## The defect this is the net under
//!
//! `do_op(Create)` skips a node that already exists. `undo_op(Create)` used to
//! remove the node unconditionally — so undoing a `Create` that created
//! nothing deleted a node somebody *else's* `Create` had made. The paper's
//! `do_undo_op_inv` has no case split on whether the op had an effect
//! (Kleppmann et al. 2022, `Move.thy`), and the reorder loop in `apply_op`
//! assumes nothing less: undo the tail, insert the late op, redo. A
//! non-inverse undo makes the redo start from the wrong tree, and the replica
//! that reordered ends up with a different tree from the one that did not.
//!
//! Two `Create` ops for one node id is not an exotic input. Page and journal
//! roots are addressed by the deterministic `NodeId::from_slug`, so two
//! devices opening the same journal offline each emit a `Create` for the
//! *same* node — the single most common duplicate in a real workspace.
//!
//! ## Relationship to the existing tests
//!
//! `create_undo_symmetry.rs` pins this deterministically: four fixed
//! reproductions plus a direct `undo_op(do_op(x)) == x` sweep over every
//! variant. Those are the regressions. This file is the *property*: a
//! generator that guarantees duplicate `Create`s at a high density, delivered
//! under many orders, with a late op forcing a full undo/redo of the log.
//!
//! The two shapes fail differently. A fixed reproduction catches the exact
//! sequence that was reported; a generator catches the neighbouring sequence
//! that nobody thought to report. Both are cheap, and the deterministic one
//! alone was not enough to find this in the first place — the convergence
//! generator was, once it stopped rewriting duplicate `Create`s into `Move`s.
//!
//! **The assertion here is the correct converged state, not today's
//! behaviour.** If it ever fails, the code is wrong, not the test.

mod convergence_gen;

use convergence_gen::{find_cycle, permutation, snapshot, Pools, Replica};
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use proptest::prelude::*;

fn pos(i: usize) -> Fractional {
    Fractional::parse(((b'a' + (i % 26) as u8) as char).to_string()).expect("valid position")
}

/// A program in which **every** node is created twice, by two different
/// actors, under two different parents — then moved around.
///
/// `convergence_gen::lower` emits duplicate `Create`s only when the random
/// program happens to draw the same node twice. This builds them on purpose,
/// which is what makes the property dense enough to shrink usefully.
fn duplicate_create_program(pools: &Pools, moves: &[(usize, usize, usize)]) -> Vec<LogOp> {
    let root = NodeId::root();
    let mut ops = Vec::new();
    let mut tick = 0u64;

    // Two Creates per node, from two actors, at different instants and under
    // different parents. The lower-HLC one owns the placement; the other must
    // be a no-op on the tree and must still be in the log.
    for (i, node) in pools.nodes.iter().enumerate() {
        let first_parent = if i == 0 { root } else { pools.nodes[i - 1] };
        ops.push(LogOp {
            ts: Hlc::new(tick, 0, pools.actors[0]),
            actor: pools.actors[0],
            op: Op::Create {
                node: *node,
                parent: first_parent,
                position: pos(i),
            },
        });
        tick += 1;
        ops.push(LogOp {
            ts: Hlc::new(tick, 0, pools.actors[1]),
            actor: pools.actors[1],
            op: Op::Create {
                node: *node,
                parent: root,
                position: pos(i + 7),
            },
        });
        tick += 1;
    }

    for (k, (n, parent, who)) in moves.iter().enumerate() {
        let node = pools.nodes[*n % pools.nodes.len()];
        let new_parent = if n == parent {
            root
        } else {
            pools.nodes[*parent % pools.nodes.len()]
        };
        let actor = pools.actors[*who % pools.actors.len()];
        ops.push(LogOp {
            ts: Hlc::new(tick, 0, actor),
            actor,
            op: Op::Move {
                node,
                new_parent,
                position: pos(k + 13),
                old_parent: root,
                old_position: Fractional::first(),
            },
        });
        tick += 1;
    }

    ops
}

fn run(ops: &[LogOp], seed: u64) -> Replica {
    let order = permutation(ops.len(), seed);
    let mut r = Replica::new(ActorId::new());
    for &i in &order {
        r.apply(ops[i].clone());
    }
    r
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// A node that receives two `Create` ops and then a reorder converges to
    /// the same tree on every replica.
    ///
    /// This is the defect's own shape. Under the broken `undo_op`, a replica
    /// that had to reorder past the second (no-op) `Create` deleted the node;
    /// a replica that received everything in order kept it. Both logs were
    /// identical, so nothing but a full-state comparison could see it.
    #[test]
    fn a_node_created_twice_converges_under_every_delivery_order(
        moves in prop::collection::vec((0usize..6, 0usize..6, 0usize..4), 0..14),
        seeds in prop::array::uniform4(any::<u64>()),
    ) {
        let pools = Pools::new();
        let ops = duplicate_create_program(&pools, &moves);

        let baseline = snapshot(&run(&ops, 1).tree);
        for seed in seeds {
            let r = run(&ops, seed);
            prop_assert_eq!(
                &snapshot(&r.tree),
                &baseline,
                "a duplicate Create diverged under delivery seed {}",
                seed
            );
            prop_assert!(find_cycle(&r.tree).is_none(), "cycle materialized (seed {})", seed);
        }
    }

    /// The same program with a **late op** appended: an op older than every
    /// `Create`, forcing `apply_op` to undo the entire log — every duplicate
    /// `Create` included — and redo it.
    ///
    /// Delivering the late op last must land on the same tree as delivering it
    /// first. That equality *is* `undo_op ∘ do_op = id` applied to a log full
    /// of no-op `Create`s, which is precisely the lemma the broken
    /// implementation violated.
    #[test]
    fn undoing_a_no_op_create_restores_the_tree_it_found(
        moves in prop::collection::vec((0usize..6, 0usize..6, 0usize..4), 0..14),
        order_seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let ops = duplicate_create_program(&pools, &moves);

        // Physical 0 is taken by the first Create, so the late op wins the
        // earliest slot via a smaller actor. If the draw does not cooperate it
        // is merely early, and the round-trip equality still holds.
        let min_pool = pools.actors.iter().min().copied().expect("actors");
        let mut late_actor = ActorId::new();
        for _ in 0..64 {
            if late_actor < min_pool {
                break;
            }
            late_actor = ActorId::new();
        }
        let late = LogOp {
            ts: Hlc::new(0, 0, late_actor),
            actor: late_actor,
            op: Op::Move {
                node: pools.nodes[0],
                new_parent: NodeId::trash(),
                position: pos(3),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        };

        let order = permutation(ops.len(), order_seed);

        // A: the whole program, then the late op — a full undo/redo sweep.
        let mut a = Replica::new(ActorId::new());
        for &i in &order {
            a.apply(ops[i].clone());
        }
        a.apply(late.clone());

        // B: the late op first — no reorder needed.
        let mut b = Replica::new(ActorId::new());
        b.apply(late.clone());
        for &i in &order {
            b.apply(ops[i].clone());
        }

        prop_assert_eq!(
            snapshot(&a.tree),
            snapshot(&b.tree),
            "undoing a no-op Create did not restore the tree it found"
        );
        prop_assert_eq!(a.log.len(), b.log.len(), "log length diverged across the reorder");
    }

    /// A duplicate `Create` must never resurrect a node that was trashed
    /// between the two `Create`s.
    ///
    /// The failure is user-visible and irreversible-looking: delete a page on
    /// the phone, and a second device's stale `Create` for the same slug
    /// brings it back. `do_op` is idempotent so it cannot resurrect on the
    /// forward path; the danger is entirely on the undo path, where removing
    /// the node and re-inserting it under the *Create's* parent would take it
    /// straight back out of the trash.
    #[test]
    fn a_duplicate_create_never_lifts_a_node_back_out_of_the_trash(
        seeds in prop::array::uniform4(any::<u64>()),
    ) {
        let pools = Pools::new();
        let node = pools.nodes[0];
        let root = NodeId::root();
        let (a1, a2) = (pools.actors[0], pools.actors[1]);

        let ops = vec![
            LogOp {
                ts: Hlc::new(10, 0, a1),
                actor: a1,
                op: Op::Create { node, parent: root, position: pos(0) },
            },
            // Trashed after the first Create…
            LogOp {
                ts: Hlc::new(20, 0, a1),
                actor: a1,
                op: Op::Move {
                    node,
                    new_parent: NodeId::trash(),
                    position: pos(1),
                    old_parent: root,
                    old_position: Fractional::first(),
                },
            },
            // …and a second device's Create for the same id lands afterwards.
            LogOp {
                ts: Hlc::new(30, 0, a2),
                actor: a2,
                op: Op::Create { node, parent: root, position: pos(2) },
            },
        ];

        for seed in seeds {
            let r = run(&ops, seed);
            prop_assert_eq!(
                r.tree.parent(node),
                Some(NodeId::trash()),
                "a duplicate Create resurrected a trashed node (seed {})",
                seed
            );
            prop_assert_eq!(r.log.len(), 3, "an op was lost (seed {})", seed);
        }
    }
}
