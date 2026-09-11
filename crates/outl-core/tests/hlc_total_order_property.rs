//! The HLC total order, **including the actor tiebreak**.
//!
//! `Hlc::cmp` is `(physical_ms, logical, actor)` lexicographic. The actor is
//! not decoration: it is what makes the order *total* rather than merely
//! partial. Two devices whose clocks agree to the millisecond and whose
//! logical counters happen to match produce timestamps that are equal in the
//! first two components, and if the comparison stopped there the two ops would
//! be unordered — every replica free to pick a different winner, every replica
//! converging to a different tree, with no test failing.
//!
//! ## Why this file exists next to `convergence_property.rs`
//!
//! That suite has `hlc_actor_tiebreak_is_deterministic`, which builds one
//! contested pair and delivers it two ways. That pins *a* tiebreak. It does
//! not pin the order's **algebra** — antisymmetry, transitivity, and the
//! equality law — and those are what a comparison rewritten for speed (a
//! packed `u128` key, a cached sort, a `PartialOrd` derived by field order)
//! actually breaks. A derived `PartialOrd` on this struct, for instance, still
//! passes a single-pair tiebreak test.
//!
//! ## The runtime caveat, stated plainly
//!
//! A separate audit of a real 217,811-op workspace measured the reorder window
//! at **0 for every op**: every path into `apply_op` currently feeds it
//! pre-sorted input, so the undo/redo machinery these orderings drive is real
//! code on a path production does not presently exercise. That is a reason to
//! pin it harder, not softer — it is the net under a planned change, and an
//! unexercised path is exactly where a regression survives review.

mod convergence_gen;

use convergence_gen::{lower, permutation, program_strategy, snapshot, Pools, Replica};
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use proptest::prelude::*;

/// Four distinct actors, sorted ascending, so a test can name "the larger
/// actor" without depending on ULID generation order.
///
/// `ActorId` is a random ULID and there is no dev-dependency on `ulid` here,
/// so the ids cannot be constructed from a literal — generating and sorting is
/// the public-API way to get a known ordering.
fn sorted_actor_pool() -> Vec<ActorId> {
    let mut actors: Vec<ActorId> = (0..4).map(|_| ActorId::new()).collect();
    actors.sort();
    actors.dedup();
    actors
}

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid position")
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
// Deterministic: the tiebreak is the actor, and it is the *last* word
// --------------------------------------------------------------------------

/// At equal physical and logical time, the larger `ActorId` sorts later —
/// and the three components are consulted in order, so a larger actor never
/// rescues a smaller physical time.
///
/// The second half is the part a packed-key rewrite gets wrong: pack the
/// fields in the wrong byte order and actor starts outranking wall clock.
#[test]
fn the_actor_is_the_last_word_in_the_order_never_the_first() {
    let actors = sorted_actor_pool();
    let (small, large) = (actors[0], actors[actors.len() - 1]);

    assert!(
        Hlc::new(10, 0, small) < Hlc::new(10, 0, large),
        "the actor tiebreak does not order equal instants"
    );
    assert!(
        Hlc::new(10, 0, large) < Hlc::new(10, 1, small),
        "the actor outranked the logical counter"
    );
    assert!(
        Hlc::new(10, 9, large) < Hlc::new(11, 0, small),
        "the actor outranked the physical clock"
    );
    assert_ne!(
        Hlc::new(10, 0, small),
        Hlc::new(10, 0, large),
        "two actors at the same instant compared equal — the order is not total"
    );
}

// --------------------------------------------------------------------------
// Properties
// --------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// The order's algebra: comparison is antisymmetric, equality means
    /// component-wise equality (all three components, actor included), and
    /// `<=` is transitive.
    ///
    /// Deliberately drawn from a *small* value space (4 physical × 3 logical ×
    /// 4 actors) so collisions on the first two components are common — that
    /// is where the tiebreak has to do its work, and a wide random space would
    /// almost never land there.
    #[test]
    fn hlc_comparison_is_a_total_order_over_physical_logical_and_actor(
        triple in prop::array::uniform3((0u64..4, 0u32..3, 0usize..4)),
    ) {
        let actors = sorted_actor_pool();
        let build = |(p, l, a): (u64, u32, usize)| Hlc::new(p, l, actors[a % actors.len()]);
        let [x, y, z] = [build(triple[0]), build(triple[1]), build(triple[2])];

        for (a, b) in [(x, y), (y, z), (x, z)] {
            prop_assert_eq!(
                a.cmp(&b),
                b.cmp(&a).reverse(),
                "comparison is not antisymmetric"
            );
            prop_assert_eq!(
                a == b,
                a.physical_ms == b.physical_ms && a.logical == b.logical && a.actor == b.actor,
                "equality disagrees with component-wise equality"
            );
        }

        // Transitivity over the three, in whichever order they sort.
        let mut sorted = [x, y, z];
        sorted.sort();
        prop_assert!(sorted[0] <= sorted[1] && sorted[1] <= sorted[2]);
        prop_assert!(sorted[0] <= sorted[2], "`<=` is not transitive");
    }

    /// Two timestamps that differ **only** in the actor are never equal, and
    /// their order follows the actors' own order.
    ///
    /// This is the property that fails the instant somebody "simplifies"
    /// `Hlc::cmp` to `(physical_ms, logical)` on the grounds that the actor is
    /// already in the op envelope.
    #[test]
    fn two_timestamps_differing_only_in_the_actor_order_by_that_actor(
        physical in 0u64..1000,
        logical in 0u32..10,
        left in 0usize..4,
        right in 0usize..4,
    ) {
        let actors = sorted_actor_pool();
        let (a, b) = (actors[left % actors.len()], actors[right % actors.len()]);
        let (x, y) = (Hlc::new(physical, logical, a), Hlc::new(physical, logical, b));

        prop_assert_eq!(x.cmp(&y), a.cmp(&b), "the tiebreak does not follow the actor order");
        prop_assert_eq!(x == y, a == b, "distinct actors produced equal timestamps");
    }

    /// Sorting a set of timestamps yields the same sequence whatever order
    /// they arrived in.
    ///
    /// The op log's whole ordering story rests on this: two devices that
    /// received the same ops over different transports, in different orders,
    /// must fold them into the identical sequence before replay.
    #[test]
    fn sorting_timestamps_gives_the_same_sequence_from_any_arrival_order(
        stamps in prop::collection::vec((0u64..6, 0u32..3, 0usize..4), 2..24),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let actors = sorted_actor_pool();
        let built: Vec<Hlc> = stamps
            .iter()
            .map(|(p, l, a)| Hlc::new(*p, *l, actors[*a % actors.len()]))
            .collect();

        let mut baseline = built.clone();
        baseline.sort();

        for seed in seeds {
            let order = permutation(built.len(), seed);
            let mut shuffled: Vec<Hlc> = order.iter().map(|&i| built[i]).collect();
            shuffled.sort();
            prop_assert_eq!(&shuffled, &baseline, "sort depends on arrival order (seed {})", seed);
        }
    }

    /// End-to-end: two ops at the *same* physical and logical instant from
    /// different actors, dropped into a random program, resolve to the same
    /// winner on every replica under every delivery order — and the winner is
    /// the larger `ActorId`.
    ///
    /// Both contested moves target a sentinel (`ROOT` / `TRASH_ROOT`), so
    /// neither can be turned into a no-op by the cycle guard. Without that the
    /// test would sometimes assert the tiebreak and sometimes assert the cycle
    /// guard, and a failure would not say which.
    #[test]
    fn ops_at_one_instant_from_two_actors_pick_the_same_winner_on_every_replica(
        program in program_strategy(),
        seeds in prop::array::uniform4(any::<u64>()),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        let contested_at = ops.len() as u64;
        let victim = pools.nodes[0];

        let actors = sorted_actor_pool();
        let (small, large) = (actors[0], actors[actors.len() - 1]);

        // Same physical, same logical, different actor. The larger actor's op
        // is later in the total order, so its target must win.
        ops.push(LogOp {
            ts: Hlc::new(contested_at, 0, small),
            actor: small,
            op: move_op(victim, NodeId::root(), "y"),
        });
        ops.push(LogOp {
            ts: Hlc::new(contested_at, 0, large),
            actor: large,
            op: move_op(victim, NodeId::trash(), "z"),
        });

        let mut baseline = None;
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
                    "the smaller actor won the tiebreak (seed {})",
                    seed
                );
            }
            let snap = snapshot(&r.tree);
            match &baseline {
                None => baseline = Some(snap),
                Some(first) => prop_assert_eq!(
                    &snap,
                    first,
                    "replicas disagreed about a contested instant (seed {})",
                    seed
                ),
            }
        }
    }
}
