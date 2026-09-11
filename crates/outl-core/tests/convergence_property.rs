//! Property-based **convergence** suite for the tree CRDT / op log.
//!
//! This is the definitive guard for outl's central correctness claim: *the
//! op log converges*. Each property generates a bounded-but-meaningful set of
//! random ops across several actors with monotonic-per-actor HLCs, then
//! delivers them to multiple replicas under random orderings (and random
//! duplication). It asserts every replica materializes the **identical** tree
//! — nodes, properties, *and* collapsed flags.
//!
//! Why a separate file from `property_based.rs`: the existing proptest suite
//! covers only `Create` + `Move` over a 5-node pool, compares only the
//! forward-vs-reversed pair, and `common::assert_trees_equal` ignores
//! property and collapsed state. This file widens the generator to the full
//! op-variant mix (`Create` / `Move` / delete=`Move`→trash / `SetProp` /
//! `SetCollapsed`, `SnoozeRemind`), asserts across *N* random permutations all-pairs-equal,
//! exercises idempotent re-delivery, builds concurrent moves that *would*
//! cycle, and compares the **full** materialized state.
//!
//! Invariants guarded (see `crates/outl-core/CLAUDE.md` → "The five
//! invariants"):
//! 1. Convergence (SEC) — all orderings agree.
//! 2. Commutativity after reordering — any permutation, not just reverse.
//! 3. Idempotency — duplicated delivery == single delivery.
//! 4. Tree invariant — no cycle ever materializes.
//! 5. No silent loss — the cycle no-op stays in every replica's log.
//!
//! Determinism: every generated `LogOp` carries a globally unique HLC
//! (`physical = step_index`, distinct actors as final tiebreak), so the
//! idempotency dedup in `Tree::apply_op` never drops two *different* ops as if
//! they were one. Proptest's RNG is seeded per case and shrinks to a minimal
//! counterexample on failure — no wall-clock, no flakiness.
//!
//! The generator these properties run on lives in `convergence_gen/`, which
//! is its single owner; the deterministic `Op::Create` regressions this suite
//! surfaced live in `create_tree_invariants.rs`. This file is only the
//! proptest properties, so `convergence_property.proptest-regressions` keeps
//! naming tests that are still here.

mod convergence_gen;

use convergence_gen::{
    cycle_dense_program_strategy, cycle_rejections, find_cycle, lower, materialize, permutation,
    program_strategy, snapshot, Pools, Replica, N_ACTORS,
};
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use proptest::prelude::*;

// --------------------------------------------------------------------------
// Properties
// --------------------------------------------------------------------------

proptest! {
    // Fixed case count + the default deterministic RNG keep this reliable in
    // CI. Bump via PROPTEST_CASES locally for extra confidence.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// PROPERTY 1 — Convergence under reordering.
    ///
    /// A random program over N actors, delivered to several replicas in
    /// several *random* permutations (not just forward/reverse), must
    /// materialize the identical full tree state on every replica, and every
    /// replica's log must hold the same number of ops.
    #[test]
    fn convergence_under_reordering(program in program_strategy(), seeds in prop::array::uniform4(any::<u64>())) {
        let pools = Pools::new();
        // Full op surface, including a Create whose parent is already a
        // descendant of the node — the cycle guard on Op::Create keeps this
        // convergent (cycle-forming Create is a tree no-op, stays in the log).
        let ops = lower(&program, &pools);

        // Baseline: textual (program) order.
        let (baseline_snap, baseline_log) = materialize(&ops);

        // Four more deliveries, each under a distinct random permutation.
        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let permuted: Vec<LogOp> = order.iter().map(|&i| ops[i].clone()).collect();
            let (snap, log_len) = materialize(&permuted);

            prop_assert_eq!(&snap, &baseline_snap, "tree diverged under reordering");
            prop_assert_eq!(log_len, baseline_log, "log length diverged under reordering");
        }
    }

    /// PROPERTY 2 — Idempotency / duplication.
    ///
    /// Delivering every op 1–3 times (P2P + iCloud can both redeliver the
    /// same op) yields the identical tree and the identical *log length* as
    /// delivering each op once. Dedup is keyed on the HLC; duplicates must
    /// vanish.
    #[test]
    fn idempotent_under_duplication(
        program in program_strategy(),
        dup_seed in any::<u64>(),
        order_seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        // Reference: each op exactly once, in a random order.
        let order = permutation(ops.len(), order_seed);
        let once: Vec<LogOp> = order.iter().map(|&i| ops[i].clone()).collect();
        let (ref_snap, ref_log) = materialize(&once);

        // Duplicated stream: replay each op 1..=3 times, interleaved by a
        // second permutation so duplicates don't arrive back-to-back.
        let mut dup_stream: Vec<LogOp> = Vec::new();
        let mut state = dup_seed | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for op in &once {
            let times = 1 + (next() % 3); // 1, 2, or 3
            for _ in 0..times {
                dup_stream.push(op.clone());
            }
        }
        // Shuffle the duplicated stream so copies are scattered.
        let dup_order = permutation(dup_stream.len(), dup_seed.rotate_left(1) | 1);
        let dup_stream: Vec<LogOp> = dup_order.iter().map(|&i| dup_stream[i].clone()).collect();

        let (dup_snap, dup_log) = materialize(&dup_stream);

        prop_assert_eq!(&dup_snap, &ref_snap, "duplication changed the tree");
        prop_assert_eq!(dup_log, ref_log, "duplication inflated the log (dedup failed)");
    }

    /// PROPERTY 3 — Concurrent moves never cycle, and converge.
    ///
    /// Concurrently move several nodes onto each other (the classic A→B,
    /// B→A; plus transitive chains). On *every* replica, regardless of
    /// delivery order: (a) no cycle materializes, (b) all replicas agree, and
    /// (c) no ops are lost — every replica's log holds all the ops (the
    /// move-that-would-cycle is a no-op on the tree but stays in the log).
    #[test]
    fn concurrent_moves_never_cycle(
        // A small set of moves drawn from a 4-node pool, biased to collide.
        moves in prop::collection::vec((0usize..4, 0usize..4, 0usize..N_ACTORS), 2..16),
        order_seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let root = NodeId::root();
        let pos = Fractional::parse("m").expect("valid position");

        // First, create all 4 nodes under root (so the moves have targets).
        let mut ops: Vec<LogOp> = Vec::new();
        for (i, n) in pools.nodes.iter().take(4).enumerate() {
            let actor = pools.actors[0];
            ops.push(LogOp {
                ts: Hlc::new(i as u64, 0, actor),
                actor,
                op: Op::Create { node: *n, parent: root, position: pos.clone() },
            });
        }
        // Then the concurrent moves. Self-moves (a==b) map to root, which is
        // harmless; a!=b builds the cycle pressure.
        let base = pools.nodes.len() as u64;
        for (k, (a, b, who)) in moves.iter().enumerate() {
            let node = pools.nodes[*a];
            let parent = if a == b { root } else { pools.nodes[*b] };
            let actor = pools.actors[*who];
            ops.push(LogOp {
                ts: Hlc::new(base + k as u64, 0, actor),
                actor,
                op: Op::Move {
                    node,
                    new_parent: parent,
                    position: pos.clone(),
                    old_parent: NodeId::root(),
                    old_position: Fractional::first(),
                },
            });
        }

        // Deliver in textual order and in a random permutation.
        let mut r1 = Replica::new(ActorId::new());
        for op in &ops {
            r1.apply(op.clone());
        }
        let order = permutation(ops.len(), order_seed);
        let mut r2 = Replica::new(ActorId::new());
        for &i in &order {
            r2.apply(ops[i].clone());
        }

        // (a) No cycle on either replica.
        prop_assert!(find_cycle(&r1.tree).is_none(), "cycle materialized (order 1)");
        prop_assert!(find_cycle(&r2.tree).is_none(), "cycle materialized (order 2)");

        // (b) Convergence.
        prop_assert_eq!(snapshot(&r1.tree), snapshot(&r2.tree), "replicas diverged");

        // (c) No silent loss: both logs hold every op (HLCs are all unique,
        // so no dedup, so log len == ops.len()).
        prop_assert_eq!(r1.log.len(), ops.len(), "ops lost from log (order 1)");
        prop_assert_eq!(r2.log.len(), ops.len(), "ops lost from log (order 2)");
    }

    /// PROPERTY 4 — HLC total order / actor tiebreak determinism.
    ///
    /// Two ops with the *same* physical+logical time but different actors must
    /// resolve to the same winner on every replica, independent of delivery
    /// order. We move the same node to two different parents at equal physical
    /// time; whichever actor wins the tiebreak must win identically on both
    /// orders.
    #[test]
    fn hlc_actor_tiebreak_is_deterministic(
        same_physical in 1u64..1000,
        order_seed in any::<u64>(),
    ) {
        // Two distinct actors; sort so we know the deterministic winner.
        let mut a1 = ActorId::new();
        let mut a2 = ActorId::new();
        if a1.0 > a2.0 {
            std::mem::swap(&mut a1, &mut a2);
        }
        // a2 has the larger ActorId → larger HLC at equal physical/logical →
        // a2's move is the later op → a2 wins.

        let node = NodeId::new();
        let p1 = NodeId::new();
        let p2 = NodeId::new();
        let root = NodeId::root();
        let pos = Fractional::parse("m").expect("valid position");

        // Setup: create node + both target parents under root, at earlier
        // physical times so they always precede the contested moves.
        let setup = vec![
            LogOp { ts: Hlc::new(0, 0, a1), actor: a1,
                op: Op::Create { node, parent: root, position: pos.clone() } },
            LogOp { ts: Hlc::new(0, 0, a2), actor: a2,
                op: Op::Create { node: p1, parent: root, position: pos.clone() } },
            LogOp { ts: Hlc::new(0, 1, a1), actor: a1,
                op: Op::Create { node: p2, parent: root, position: pos.clone() } },
        ];

        // The two contested moves: same physical+logical, different actor.
        let move_a1 = LogOp { ts: Hlc::new(same_physical, 0, a1), actor: a1,
            op: Op::Move { node, new_parent: p1, position: pos.clone(),
                old_parent: NodeId::root(), old_position: Fractional::first() } };
        let move_a2 = LogOp { ts: Hlc::new(same_physical, 0, a2), actor: a2,
            op: Op::Move { node, new_parent: p2, position: pos.clone(),
                old_parent: NodeId::root(), old_position: Fractional::first() } };

        // Two deliveries: a1-then-a2, and a2-then-a1, each after random setup.
        let build = |contested: [&LogOp; 2], seed: u64| -> NodeId {
            let mut all = setup.clone();
            all.push(contested[0].clone());
            all.push(contested[1].clone());
            let order = permutation(all.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(all[i].clone());
            }
            r.tree.parent(node).expect("node exists")
        };

        let winner_fwd = build([&move_a1, &move_a2], order_seed);
        let winner_rev = build([&move_a2, &move_a1], order_seed.rotate_left(17) | 1);

        // a2 has the larger ActorId, so its move (equal physical/logical) is
        // the later op in the total order → a2 wins → parent == p2.
        prop_assert_eq!(winner_fwd, p2, "tiebreak winner wrong (delivery 1)");
        prop_assert_eq!(winner_rev, p2, "tiebreak winner wrong (delivery 2)");
    }

    /// PROPERTY 5 — Undo/redo round-trip (the reorder mechanism).
    ///
    /// outl has no user-facing undo; `undo_op`/`do_op` exist as the engine
    /// `apply_op` uses to reorder when a late op (smaller HLC than the log
    /// tail) arrives. The convergence properties above already exercise that
    /// path heavily. This property pins the *round-trip* directly: applying a
    /// program, then delivering one extra op with an HLC *older* than the
    /// whole log (forcing a full undo→redo of every existing op), converges
    /// to the same state as delivering that op first. i.e. the undo/redo of
    /// the entire log is a faithful round-trip.
    #[test]
    fn late_op_undo_redo_round_trips(program in program_strategy(), late_seed in any::<u64>()) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        // Construct a "late" op: a Move of some node, stamped with a physical
        // time *below* every op in `ops` (which start at physical 0), using a
        // dedicated actor and a negative-most logical slot. Since physical 0
        // is the floor, we give the late op a smaller ActorId tiebreak at
        // physical 0 — guaranteeing it sorts before the program's first op.
        let late_actor = {
            // Find an ActorId strictly smaller than every pool actor so the
            // late op wins the "earliest" slot deterministically.
            let min_pool = pools.actors.iter().map(|a| a.0).min().expect("actors");
            // Generate until we get one below min_pool (ULID space is huge;
            // in practice the first random one usually qualifies, but loop to
            // be safe and deterministic-enough for a test).
            let mut candidate = ActorId::new();
            let mut tries = 0;
            while candidate.0 >= min_pool && tries < 64 {
                candidate = ActorId::new();
                tries += 1;
            }
            candidate
        };
        // If we couldn't find a smaller actor (astronomically unlikely),
        // fall back to physical underflow guard: just skip the assertion's
        // "late" guarantee by using physical 0 with the found actor; the
        // round-trip equality still holds regardless of who is earliest.
        let pos = Fractional::parse("m").expect("valid position");
        let late = LogOp {
            ts: Hlc::new(0, 0, late_actor),
            actor: late_actor,
            op: Op::Move {
                node: pools.nodes[0],
                new_parent: NodeId::root(),
                position: pos,
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        };

        // Delivery A: full program (random order) THEN the late op. This
        // forces apply_op to undo the entire log and redo it around `late`.
        let order = permutation(ops.len(), late_seed);
        let mut a = Replica::new(ActorId::new());
        for &i in &order {
            a.apply(ops[i].clone());
        }
        a.apply(late.clone());

        // Delivery B: the late op FIRST, then the program. No reorder needed.
        let mut b = Replica::new(ActorId::new());
        b.apply(late.clone());
        for &i in &order {
            b.apply(ops[i].clone());
        }

        prop_assert_eq!(
            snapshot(&a.tree),
            snapshot(&b.tree),
            "undo/redo round-trip diverged from late-first delivery"
        );
        prop_assert_eq!(a.log.len(), b.log.len(), "log length diverged across undo/redo");
    }
}

// --------------------------------------------------------------------------
// Properties 6-8 — the same claims, over programs the cycle guard rejects.
//
// Properties 1, 2 and 5 above are the broad convergence assertions, and they
// are made almost entirely about programs in which the cycle guard never
// fired: 41 of 400 generated programs contain a single rejection, 1.22% of
// their structural ops (re-derived by `cycle_rejections`, which asks
// `Tree::creates_cycle` before each apply rather than inferring rejection
// from the final tree). Root `CLAUDE.md` invariant 4 — the rejected op is a
// no-op on the tree but stays in the log, "removing it breaks correctness of
// future reordering" — is therefore pinned by `concurrent_moves_never_cycle`
// and little else, and that property is Move-only, compares two delivery
// orders, and never sees a property or a collapsed flag.
//
// These three re-make properties 1, 2 and 5 over `cycle_dense_program_strategy()`
// instead. Nothing above is weakened or replaced: the broad generator keeps
// its breadth, and this one buys density next to it.
// --------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// PROPERTY 6 — Convergence still holds when the guard rejects often.
    ///
    /// Property 1's claim (N random permutations all materialize the identical
    /// full state, and every log holds the same op count) over a program built
    /// so a large share of its moves are cycle no-ops. A rejected move is
    /// invisible in the materialized state, so a replica that dropped one from
    /// its log diverges only once a later reorder would have revived it — this
    /// is the shape of delivery order that reaches that.
    #[test]
    fn convergence_holds_when_the_cycle_guard_rejects_often(
        program in cycle_dense_program_strategy(),
        seeds in prop::array::uniform4(any::<u64>()),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        let (baseline_snap, baseline_log) = materialize(&ops);

        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let permuted: Vec<LogOp> = order.iter().map(|&i| ops[i].clone()).collect();
            let (snap, log_len) = materialize(&permuted);

            prop_assert_eq!(&snap, &baseline_snap, "tree diverged under reordering");
            prop_assert_eq!(log_len, baseline_log, "log length diverged under reordering");
        }
    }

    /// PROPERTY 7 — A rejected move survives the undo/redo reorder.
    ///
    /// Property 5's late-op round trip over the same dense programs. This is
    /// the interaction invariant 4's second clause is about: `undo_op` reverts
    /// a `Move` only when the node's current parent equals the move's
    /// `new_parent`, which is exactly how it tells a move that took effect
    /// from one the guard rejected. A late op forces every op in the log back
    /// through that discrimination and then through `do_op` again, where a
    /// move rejected the first time can now be legal (and vice versa).
    /// Delivering the late op first needs no reorder at all, so the two must
    /// agree — and they can only agree if the rejected ops were still there to
    /// redo.
    #[test]
    fn a_move_the_guard_rejected_is_redone_faithfully_after_a_late_op(
        program in cycle_dense_program_strategy(),
        late_seed in any::<u64>(),
        late_node in 0usize..5,
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        // An actor id below every pool actor so the op sorts before the whole
        // program at physical 0 and forces a full undo of the log. Same
        // construction as `late_op_undo_redo_round_trips`; the fallback after
        // 64 tries costs the "earliest" guarantee, never the equality claim.
        let late_actor = {
            let min_pool = pools.actors.iter().map(|a| a.0).min().expect("actors");
            let mut candidate = ActorId::new();
            let mut tries = 0;
            while candidate.0 >= min_pool && tries < 64 {
                candidate = ActorId::new();
                tries += 1;
            }
            candidate
        };

        // The late op moves a chain node under a *deeper* chain node, i.e. it
        // is itself a likely cycle — the case where the reorder mechanism and
        // the guard have to agree.
        let deeper = (late_node + 1) % 5;
        let late = LogOp {
            ts: Hlc::new(0, 0, late_actor),
            actor: late_actor,
            op: Op::Move {
                node: pools.nodes[late_node],
                new_parent: pools.nodes[deeper],
                position: Fractional::parse("m").expect("valid position"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        };

        let order = permutation(ops.len(), late_seed);

        let mut a = Replica::new(ActorId::new());
        for &i in &order {
            a.apply(ops[i].clone());
        }
        a.apply(late.clone());

        let mut b = Replica::new(ActorId::new());
        b.apply(late.clone());
        for &i in &order {
            b.apply(ops[i].clone());
        }

        prop_assert!(find_cycle(&a.tree).is_none(), "cycle materialized after reorder");
        prop_assert!(find_cycle(&b.tree).is_none(), "cycle materialized without reorder");
        prop_assert_eq!(
            snapshot(&a.tree),
            snapshot(&b.tree),
            "undo/redo round-trip diverged from late-first delivery"
        );
        prop_assert_eq!(a.log.len(), b.log.len(), "log length diverged across undo/redo");
    }

    /// PROPERTY 8 — Duplicated delivery of a rejected move changes nothing.
    ///
    /// Property 2's claim over the dense programs. Dedup is keyed on the HLC
    /// and a rejected move mutates nothing, so a dedup that keyed on effect
    /// instead — "this op did nothing, drop it" — passes property 2 and fails
    /// here the moment a reorder would have made the dropped copy matter.
    #[test]
    fn duplicating_a_rejected_move_leaves_the_tree_and_the_log_alone(
        program in cycle_dense_program_strategy(),
        dup_seed in any::<u64>(),
        order_seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        let order = permutation(ops.len(), order_seed);
        let once: Vec<LogOp> = order.iter().map(|&i| ops[i].clone()).collect();
        let (ref_snap, ref_log) = materialize(&once);

        let mut state = dup_seed | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut dup_stream: Vec<LogOp> = Vec::new();
        for op in &once {
            let times = 1 + (next() % 3);
            for _ in 0..times {
                dup_stream.push(op.clone());
            }
        }
        let dup_order = permutation(dup_stream.len(), dup_seed.rotate_left(1) | 1);
        let dup_stream: Vec<LogOp> = dup_order.iter().map(|&i| dup_stream[i].clone()).collect();

        let (dup_snap, dup_log) = materialize(&dup_stream);

        prop_assert_eq!(&dup_snap, &ref_snap, "duplication changed the tree");
        prop_assert_eq!(dup_log, ref_log, "duplication inflated the log (dedup failed)");
    }
}

// --------------------------------------------------------------------------
// The coverage claim itself, made fail-able.
// --------------------------------------------------------------------------

/// The dense generator is only worth its three properties while it is still
/// dense, and nothing about a proptest run reports that. A later edit to
/// `cycle_dense_step` — reweighting the kinds, widening the node pool,
/// letting `Delete` flatten the chain — can quietly return it to the broad
/// generator's rate, and every property above would keep passing while
/// covering nothing.
///
/// So the density is asserted rather than assumed. It is stated as two
/// absolute floors plus one comparison, because the comparison alone is a
/// ratio of two sampled rates and the broad generator's rate is small enough
/// (2.4-2.9% across runs) to move the ratio by a third on noise alone.
///
/// Measured at the time of writing, over 200 sampled programs each:
///
/// | generator                        | rejected / consulted | programs hit |
/// |----------------------------------|----------------------|--------------|
/// | `program_strategy`               | ~2.5%                | ~10%         |
/// | `cycle_dense_program_strategy`   | ~22.7%               | 100%         |
///
/// The floors sit well below those, so this pins the claim without becoming a
/// re-measurement that fails on sampling noise.
#[test]
fn the_cycle_dense_generator_rejects_far_more_moves_than_the_shared_one() {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::{Config, TestRunner};

    const SAMPLES: usize = 200;

    fn sample(strategy: impl Strategy<Value = Vec<convergence_gen::Step>>) -> (f64, usize) {
        let mut runner = TestRunner::new(Config::default());
        let (mut rejected, mut consulted, mut programs_hit) = (0usize, 0usize, 0usize);
        for _ in 0..SAMPLES {
            let program = strategy
                .new_tree(&mut runner)
                .expect("generator produced a value")
                .current();
            let ops = lower(&program, &Pools::new());
            let (r, c) = cycle_rejections(&ops);
            rejected += r;
            consulted += c;
            if r > 0 {
                programs_hit += 1;
            }
        }
        (rejected as f64 / consulted.max(1) as f64, programs_hit)
    }

    let (broad_rate, broad_hit) = sample(program_strategy());
    let (dense_rate, dense_hit) = sample(cycle_dense_program_strategy());

    assert!(
        dense_rate >= 0.15,
        "the cycle guard now rejects only {:.1}% of the dense generator's \
         structural ops (was ~22.7%); its three properties are no longer \
         covering the path they exist for",
        dense_rate * 100.0
    );
    assert!(
        dense_hit * 10 >= SAMPLES * 9,
        "only {dense_hit}/{SAMPLES} dense programs reject an op (was all of \
         them); the broad generator manages {broad_hit}/{SAMPLES}, so this one \
         is buying nothing"
    );
    assert!(
        dense_rate >= 5.0 * broad_rate,
        "the dense generator is no longer denser than the shared one it exists \
         to complement: {:.1}% vs {:.1}%",
        dense_rate * 100.0,
        broad_rate * 100.0
    );
}
