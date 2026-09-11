//! `undo_op` must be the exact inverse of `do_op` — for *every* op, including
//! the ones `do_op` turned into no-ops.
//!
//! This is the hinge of the whole convergence argument. In the authors'
//! Isabelle development it is the lemma
//!
//! ```text
//! lemma do_undo_op_inv:
//!   assumes ‹unique_parent tree›
//!   shows   ‹undo_op (do_op (Move t p m c, tree)) = tree›
//! ```
//!
//! (`proof/Move.thy`), whose **only** hypothesis is that the tree is
//! well-formed — there is no case split on whether the op had an effect. It
//! is invoked five times inside the commutativity proof that discharges
//! `theorem apply_ops_commutes` (paper §4.2), which is what plugs the
//! algorithm into Gomes et al.'s SEC framework. Break the lemma and strong
//! eventual consistency is gone.
//!
//! outl's [`outl_core::op::Op::Create`] broke it. `do_op` is idempotent —
//! it skips a node that already exists — while `undo_op` removed the node
//! unconditionally, so undoing a `Create` that created nothing deleted a
//! node somebody *else's* `Create` had made. See
//! `docs/rfcs/0263-create-is-invertible.md`.

mod convergence_gen;

// `snapshot` is the battery's single owner of "the full materialized state,
// canonically ordered" — `Tree` has no `PartialEq` (and should not: comparing
// two trees is a test concern). A second copy here would be free to drift
// into comparing less, which is exactly how this defect stayed invisible.
use convergence_gen::snapshot;
use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::log::OpLog;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::tree::Tree;

fn at(physical_ms: u64, actor: ActorId, op: Op) -> LogOp {
    let ts = Hlc::new(physical_ms, 0, actor);
    LogOp { ts, actor, op }
}

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid fractional position")
}

fn run(ops: &[LogOp], order: &[usize]) -> Tree {
    let mut tree = Tree::new();
    let mut log = OpLog::new();
    for &i in order {
        tree.apply_op(&mut log, ops[i].clone());
    }
    tree
}

// --------------------------------------------------------------------------
// The defect, minimally.
// --------------------------------------------------------------------------

/// Two `Create`s for one node, with a `Move` in between, must converge.
///
/// This is not an exotic input. Page and journal roots are addressed by
/// [`NodeId::from_slug`], a *deterministic* hash of the slug, precisely so
/// that two devices which create the same page offline land on one node
/// rather than two competing roots (`outl_actions::page::open_or_create`).
/// The cost of that design is that a duplicate `Op::Create` for a single
/// node id is **routine** — every journal opened on two devices produces
/// one — and each carries whatever sibling position its own device
/// computed.
#[test]
fn a_repeat_create_converges_under_reordering() {
    let actor = ActorId::new();
    let n = NodeId::from_slug("2026-09-11");
    let parent = NodeId::root();

    let ops = vec![
        at(
            1,
            actor,
            Op::Create {
                node: n,
                parent,
                position: pos("a"),
            },
        ),
        at(
            2,
            actor,
            Op::Move {
                node: n,
                new_parent: parent,
                position: pos("t"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        at(
            3,
            actor,
            Op::Create {
                node: n,
                parent,
                position: pos("u"),
            },
        ),
    ];

    let in_order = run(&ops, &[0, 1, 2]);
    let reordered = run(&ops, &[2, 0, 1]);

    assert_eq!(
        snapshot(&in_order),
        snapshot(&reordered),
        "delivery order changed the materialized tree: HLC order says the \
         node sits where Move@2 put it, and the second Create is an \
         idempotent no-op. Undoing that no-op must not delete the node."
    );
}

/// The same defect, stated as what it costs the user: the `Move` is not
/// merely re-ordered, it is **lost**.
///
/// After the spurious `undo_op(Create)` removes the node, `do_op(Move)`
/// finds nothing to move (outl's `Move` is not total over unborn nodes) and
/// becomes a complete no-op. The replay of the later `Create` then
/// re-materializes the node at the *creation* position. The op stays in the
/// log — invariant 5 holds — but its effect is gone from the tree, which is
/// the silent-loss shape invariant 1 exists to prevent.
#[test]
fn a_repeat_create_does_not_swallow_an_interleaved_move() {
    let actor = ActorId::new();
    let n = NodeId::new();
    let parent = NodeId::root();

    let ops = vec![
        at(
            1,
            actor,
            Op::Create {
                node: n,
                parent,
                position: pos("a"),
            },
        ),
        at(
            2,
            actor,
            Op::Move {
                node: n,
                new_parent: parent,
                position: pos("t"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        at(
            3,
            actor,
            Op::Create {
                node: n,
                parent,
                position: pos("u"),
            },
        ),
    ];

    let reordered = run(&ops, &[2, 0, 1]);
    assert_eq!(
        reordered.position(n).map(|p| p.as_str()),
        Some("t"),
        "the Move's effect must survive an out-of-order duplicate Create"
    );
}

/// Every permutation of a duplicate-`Create` program agrees.
///
/// Three ops means six orderings; the CRDT claim is that the *set* of ops
/// determines the state, so all six must land on one tree.
#[test]
fn every_delivery_order_of_a_repeat_create_agrees() {
    let a = ActorId::new();
    let b = ActorId::new();
    let n = NodeId::from_slug("ideas");
    let parent = NodeId::root();

    let ops = vec![
        at(
            10,
            a,
            Op::Create {
                node: n,
                parent,
                position: pos("a"),
            },
        ),
        at(
            20,
            b,
            Op::Move {
                node: n,
                new_parent: NodeId::trash(),
                position: pos("t"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        at(
            30,
            b,
            Op::Create {
                node: n,
                parent,
                position: pos("u"),
            },
        ),
    ];

    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let reference = snapshot(&run(&ops, &orders[0]));
    for order in &orders[1..] {
        assert_eq!(
            snapshot(&run(&ops, order)),
            reference,
            "delivery order {order:?} diverged"
        );
    }
}

/// A duplicate `Create` must not resurrect a trashed node.
///
/// Device A creates page `ideas` and deletes it (`Move` → trash). Device B,
/// which never saw either op, creates `ideas` locally — the same node id, by
/// [`NodeId::from_slug`]. Once they sync, HLC order puts B's `Create` last,
/// where it is an idempotent no-op, so the page stays deleted on both. The
/// wrong answer here is not a crash: it is a page that reappears on one
/// device and not the other.
#[test]
fn a_duplicate_create_does_not_resurrect_a_trashed_node() {
    let a = ActorId::new();
    let b = ActorId::new();
    let n = NodeId::from_slug("ideas");

    let ops = vec![
        at(
            1,
            a,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: pos("a"),
            },
        ),
        at(
            5,
            a,
            Op::Move {
                node: n,
                new_parent: NodeId::trash(),
                position: pos("a"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        at(
            9,
            b,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: pos("a"),
            },
        ),
    ];

    let in_order = run(&ops, &[0, 1, 2]);
    assert_eq!(
        in_order.parent(n),
        Some(NodeId::trash()),
        "HLC order: the late Create is a no-op, the node stays trashed"
    );
    for order in [[2, 0, 1], [1, 2, 0], [2, 1, 0]] {
        assert_eq!(
            snapshot(&run(&ops, &order)),
            snapshot(&in_order),
            "delivery order {order:?} diverged"
        );
    }
}

// --------------------------------------------------------------------------
// The general property: `undo_op ∘ do_op == id`, for every variant.
//
// Named after the property rather than the repro on purpose. A per-variant
// test keeps missing the next variant — the same argument
// `tests/stored_op_matches_the_log.rs` makes about `old_*` fields.
// --------------------------------------------------------------------------

/// Build a small but structurally varied tree, plus the ids it uses.
fn seeded_tree() -> (Tree, Vec<NodeId>) {
    let actor = ActorId::new();
    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let ops = vec![
        at(
            1,
            actor,
            Op::Create {
                node: a,
                parent: NodeId::root(),
                position: pos("a"),
            },
        ),
        at(
            2,
            actor,
            Op::Create {
                node: b,
                parent: a,
                position: pos("b"),
            },
        ),
        at(
            3,
            actor,
            Op::Create {
                node: c,
                parent: b,
                position: pos("c"),
            },
        ),
        at(
            4,
            actor,
            Op::SetProp {
                node: a,
                key: "title".into(),
                value: Some(PropValue::Text("seed".into())),
                old_value: None,
            },
        ),
        at(
            5,
            actor,
            Op::SetCollapsed {
                node: b,
                value: true,
                old_value: false,
            },
        ),
        at(
            6,
            actor,
            Op::SnoozeRemind {
                node: c,
                until_ms: Some(1_000),
                old_until_ms: None,
            },
        ),
    ];
    let order: Vec<usize> = (0..ops.len()).collect();
    (run(&ops, &order), vec![a, b, c])
}

#[test]
fn undo_op_is_the_inverse_of_do_op_for_every_op() {
    let (tree, ids) = seeded_tree();
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let fresh = NodeId::new();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    // Each case is (label, op). The set deliberately covers, for every
    // variant, both the "had an effect" and the "was a no-op" paths — the
    // lemma has no hypothesis that the op did anything.
    let cases: Vec<(&str, Op)> = vec![
        // Create: inserts.
        (
            "create/inserts",
            Op::Create {
                node: fresh,
                parent: NodeId::root(),
                position: pos("m"),
            },
        ),
        // Create: node already exists — the defect this file exists for.
        (
            "create/node-exists",
            Op::Create {
                node: a,
                parent: NodeId::root(),
                position: pos("z"),
            },
        ),
        // Create: node already exists, under a *different* parent.
        (
            "create/node-exists-elsewhere",
            Op::Create {
                node: c,
                parent: NodeId::root(),
                position: pos("z"),
            },
        ),
        // Create: would close a cycle (`b` is already an ancestor of `c`).
        (
            "create/cycle",
            Op::Create {
                node: b,
                parent: c,
                position: pos("m"),
            },
        ),
        // Move: ordinary.
        (
            "move/effective",
            Op::Move {
                node: c,
                new_parent: NodeId::root(),
                position: pos("m"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        // Move: cycle no-op.
        (
            "move/cycle",
            Op::Move {
                node: a,
                new_parent: c,
                position: pos("m"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        // Move: node not in the tree — a complete no-op.
        (
            "move/unknown-node",
            Op::Move {
                node: fresh,
                new_parent: NodeId::root(),
                position: pos("m"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        ),
        // SetProp: overwrite, set-new, and clear.
        (
            "setprop/overwrite",
            Op::SetProp {
                node: a,
                key: "title".into(),
                value: Some(PropValue::Text("changed".into())),
                old_value: None,
            },
        ),
        (
            "setprop/new",
            Op::SetProp {
                node: b,
                key: "kind".into(),
                value: Some(PropValue::Text("page".into())),
                old_value: None,
            },
        ),
        (
            "setprop/clear",
            Op::SetProp {
                node: a,
                key: "title".into(),
                value: None,
                old_value: None,
            },
        ),
        (
            "setprop/clear-unset",
            Op::SetProp {
                node: c,
                key: "absent".into(),
                value: None,
                old_value: None,
            },
        ),
        // SetCollapsed: both directions, including the redundant one.
        (
            "setcollapsed/fold",
            Op::SetCollapsed {
                node: a,
                value: true,
                old_value: false,
            },
        ),
        (
            "setcollapsed/unfold",
            Op::SetCollapsed {
                node: b,
                value: false,
                old_value: false,
            },
        ),
        (
            "setcollapsed/redundant",
            Op::SetCollapsed {
                node: b,
                value: true,
                old_value: false,
            },
        ),
        // SnoozeRemind: set, overwrite, clear.
        (
            "snooze/set",
            Op::SnoozeRemind {
                node: a,
                until_ms: Some(9_000),
                old_until_ms: None,
            },
        ),
        (
            "snooze/overwrite",
            Op::SnoozeRemind {
                node: c,
                until_ms: Some(2_000),
                old_until_ms: None,
            },
        ),
        (
            "snooze/clear",
            Op::SnoozeRemind {
                node: c,
                until_ms: None,
                old_until_ms: None,
            },
        ),
        // Edit: a tree-level no-op on both sides, by construction.
        (
            "edit",
            Op::Edit {
                node: a,
                text_op: vec![1, 2, 3],
            },
        ),
    ];

    // Guard the guard: a variant missing from `cases` would make the
    // assertion below pass without ever looking at it. Same argument as
    // `op.rs`'s `every_variant_has_a_sample` — the lemma is about *every*
    // op, so a hand-written case list needs something that fails when the
    // enum grows.
    let covered: std::collections::HashSet<_> = cases
        .iter()
        .map(|(_, op)| std::mem::discriminant(op))
        .collect();
    assert_eq!(
        covered.len(),
        6,
        "Op has a variant with no case here. `undo_op ∘ do_op == id` is \
         claimed for every op, so a new variant needs at least one case \
         where it has an effect and one where do_op ignores it — see \
         docs/rfcs/0263-create-is-invertible.md",
    );

    let before = snapshot(&tree);
    for (label, op) in cases {
        let mut scratch = tree.clone();
        let mut log_op = LogOp {
            ts: g.next(),
            actor,
            op,
        };
        scratch.do_op(&mut log_op);
        scratch.undo_op(&log_op);
        assert_eq!(
            snapshot(&scratch),
            before,
            "undo_op(do_op(op)) != id for case {label:?} — this is the \
             lemma `do_undo_op_inv` (Kleppmann et al. 2022, Move.thy), \
             which the commutativity proof behind SEC depends on"
        );
    }
}
