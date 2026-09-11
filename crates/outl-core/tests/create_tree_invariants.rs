//! Deterministic regressions for the tree invariant around `Op::Create`.
//!
//! Both were surfaced by the convergence property battery and then pinned as
//! fixed reproductions, which is why they live next to it rather than in
//! `tree_unit.rs`: they share its generator module (`convergence_gen/`) and
//! its vocabulary. Split out of `convergence_property.rs` at 838 lines,
//! against a 900-line hard stop — the tests kept their names, so
//! `cargo test create_respects_cycle_guard` and the citations in
//! `crates/outl-core/CLAUDE.md` still resolve.
//!
//! The *undo* half of `Op::Create`'s contract — that `undo_op` is the exact
//! inverse of `do_op` — is `create_undo_symmetry.rs`.

mod convergence_gen;

use convergence_gen::{find_cycle, permutation, Replica};
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};

// --------------------------------------------------------------------------
// Regression: Op::Create must honor the cycle guard (was a real bug, found by
// the convergence suite — see crates/outl-core/CLAUDE.md).
// --------------------------------------------------------------------------

/// `Op::Create` must run the cycle guard, exactly like `Op::Move`.
///
/// `Tree::do_op`'s `Op::Move` branch calls `creates_cycle` before re-parenting.
/// The `Op::Create` branch must do the same: a `Create(node, parent)` whose
/// `parent` is already a descendant of `node` would insert an edge
/// `node → parent` that closes a loop, violating invariant #4 ("materialized
/// state is always a valid tree") and later panicking `creates_cycle` on the
/// malformed tree (`debug_assert!` in `src/tree/cycle.rs`).
///
/// This was a real bug (`Op::Create` did a bare `or_insert`); it is order-
/// independent and reproduces in *every* delivery order. The fix makes a
/// cycle-forming Create a no-op on the tree (the op still goes into the log),
/// the same way Move handles it.
///
/// Minimal reproduction (single actor):
///
/// 1. `Create(B, root)` — B exists under root.
/// 2. `Move(B, C)` — C is a phantom (no node record yet), so `creates_cycle`
///    walks from C, hits `parent(C) == None`, returns `false`, and sets B → C.
/// 3. `Create(C, B)` — `creates_cycle(C, B)` now walks B → C, hits C == node,
///    returns `true`, so the Create is a tree no-op. C is never materialized;
///    B keeps its phantom parent C. No cycle.
#[test]
fn create_respects_cycle_guard() {
    let actor = ActorId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let root = NodeId::root();
    let p = Fractional::parse("m").expect("valid position");

    let ops = [
        LogOp {
            ts: Hlc::new(1, 0, actor),
            actor,
            op: Op::Create {
                node: b,
                parent: root,
                position: p.clone(),
            },
        },
        LogOp {
            ts: Hlc::new(2, 0, actor),
            actor,
            op: Op::Move {
                node: b,
                new_parent: c, // C is still a phantom -> cycle guard sees no loop
                position: p.clone(),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        },
        LogOp {
            ts: Hlc::new(3, 0, actor),
            actor,
            op: Op::Create {
                node: c,
                parent: b, // would close B -> C -> B; the guard makes it a no-op
                position: p,
            },
        },
    ];

    // Apply in every permutation: apply_op orders by HLC, so the materialized
    // result must be identical and cycle-free regardless of delivery order.
    for seed in [1u64, 2, 3, 5, 8, 13, 21, 34] {
        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(actor);
        for &i in &order {
            r.apply(ops[i].clone());
        }

        assert!(
            find_cycle(&r.tree).is_none(),
            "Op::Create closed a cycle the Move opened (seed {seed}): parent(B)={:?}, parent(C)={:?}",
            r.tree.parent(b),
            r.tree.parent(c),
        );
        // The cycle-forming Create is a no-op: C is never materialized, and B
        // keeps the phantom parent the Move gave it. All three ops still logged.
        assert_eq!(r.tree.parent(b), Some(c), "B should keep its Move target C");
        assert_eq!(
            r.tree.parent(c),
            None,
            "C must not be materialized (Create was a cycle no-op)"
        );
        assert_eq!(r.log.len(), 3, "every op stays in the log (no silent loss)");
    }
}

/// A `Create` whose `parent` does not exist yet still materializes the node —
/// the parent is a phantom, exactly like a `Move` that arrives before its
/// target's `Create` (the `None` arm of `do_op`'s Move branch). The cycle guard
/// must NOT mistake an absent parent for a cycle: `creates_cycle` walking from a
/// parent with no record hits `None` and returns `false`, so the edge is laid
/// down. When the parent's own `Create` arrives, the chain resolves with no
/// cycle, and both ops are in the log — in every delivery order.
#[test]
fn create_with_phantom_parent_materializes_then_resolves() {
    let actor = ActorId::new();
    let x = NodeId::new();
    let parent = NodeId::new();
    let root = NodeId::root();
    let pos = Fractional::parse("m").expect("valid position");

    let ops = [
        // Create(X, parent) — `parent` has no record yet (phantom).
        LogOp {
            ts: Hlc::new(1, 0, actor),
            actor,
            op: Op::Create {
                node: x,
                parent,
                position: pos.clone(),
            },
        },
        // Create(parent, root) — materializes the phantom under root.
        LogOp {
            ts: Hlc::new(2, 0, actor),
            actor,
            op: Op::Create {
                node: parent,
                parent: root,
                position: pos,
            },
        },
    ];

    for seed in [1u64, 2, 7, 11] {
        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(actor);
        for &i in &order {
            r.apply(ops[i].clone());
        }

        assert!(
            find_cycle(&r.tree).is_none(),
            "phantom-parent Create must not cycle (seed {seed})"
        );
        assert_eq!(
            r.tree.parent(x),
            Some(parent),
            "X stays parented under the (now real) parent"
        );
        assert_eq!(
            r.tree.parent(parent),
            Some(root),
            "parent resolves under root once its Create lands"
        );
        assert_eq!(r.log.len(), 2, "both ops stay in the log");
    }
}
