//! Two replicas inserting siblings into the same gap must converge.
//!
//! Each replica picks its own midpoint and emits a `Move` with that
//! `Fractional`. The HLC total order then decides the final sibling order
//! on every replica.

mod common;

use common::{assert_trees_equal, create_op, move_op, op_at, pos, Replica};
use outl_core::fractional::Fractional;
use outl_core::id::{ActorId, NodeId};
use proptest::prelude::*;

#[test]
fn concurrent_inserts_same_gap_converge() {
    let actor1 = ActorId::new();
    let actor2 = ActorId::new();
    let root = NodeId::root();
    let parent = NodeId::new();
    let left_sibling = NodeId::new();
    let right_sibling = NodeId::new();
    let new1 = NodeId::new();
    let new2 = NodeId::new();

    let setup = [
        op_at(actor1, 1, 0, create_op(parent, root, pos("a"))),
        op_at(
            actor1,
            2,
            0,
            create_op(left_sibling, parent, Fractional::parse("a").unwrap()),
        ),
        op_at(
            actor1,
            3,
            0,
            create_op(right_sibling, parent, Fractional::parse("z").unwrap()),
        ),
    ];

    // Both replicas independently pick the midpoint between "a" and "z".
    let mid = Fractional::between(
        Some(&Fractional::parse("a").unwrap()),
        Some(&Fractional::parse("z").unwrap()),
    );

    let insert1 = op_at(actor1, 10, 0, create_op(new1, parent, mid.clone()));
    let insert2 = op_at(actor2, 11, 0, create_op(new2, parent, mid.clone()));

    let mut r1 = Replica::new(actor1);
    let mut r2 = Replica::new(actor2);
    for op in &setup {
        r1.apply(op.clone());
        r2.apply(op.clone());
    }

    r1.apply(insert1.clone());
    r1.apply(insert2.clone());

    // r2 applies in opposite order.
    r2.apply(insert2);
    r2.apply(insert1);

    assert_trees_equal(&r1.tree, &r2.tree);

    // Both siblings exist in both replicas.
    assert_eq!(r1.tree.parent(new1), Some(parent));
    assert_eq!(r1.tree.parent(new2), Some(parent));
}

#[test]
fn fifty_inserts_in_same_gap_remain_distinct() {
    // Simulates rapid sibling insertions before subsequent siblings can
    // be "compressed". Each insert picks midpoint between the previous
    // insert and the right sibling. After 50 inserts the position
    // strings are longer but always distinct.
    let actor = ActorId::new();
    let mut r = Replica::new(actor);
    let parent = NodeId::new();
    let right = NodeId::new();

    r.apply(op_at(
        actor,
        1,
        0,
        create_op(parent, NodeId::root(), pos("a")),
    ));
    r.apply(op_at(
        actor,
        2,
        0,
        create_op(right, parent, Fractional::parse("z").unwrap()),
    ));

    let mut positions: Vec<Fractional> = Vec::new();
    let mut last = Fractional::parse("a").unwrap();
    let right_pos = Fractional::parse("z").unwrap();
    for i in 0..50 {
        let m = Fractional::between(Some(&last), Some(&right_pos));
        let n = NodeId::new();
        r.apply(op_at(
            actor,
            (i as u64) + 100,
            0,
            create_op(n, parent, m.clone()),
        ));
        positions.push(m.clone());
        last = m;
    }

    // All 50 positions are unique.
    let unique: std::collections::HashSet<_> = positions.iter().collect();
    assert_eq!(unique.len(), 50);

    // All positions are strictly between "a" and "z".
    let a = Fractional::parse("a").unwrap();
    for p in &positions {
        assert!(p > &a, "{p} not > a");
        assert!(p < &right_pos, "{p} not < z");
    }
}

#[test]
fn move_preserves_position_of_unaffected_siblings() {
    // Moving one sibling does not change the position of others — a key
    // property that makes fractional indexing cheaper than re-numbering.
    let actor = ActorId::new();
    let parent = NodeId::new();
    let s1 = NodeId::new();
    let s2 = NodeId::new();
    let s3 = NodeId::new();

    let mut r = Replica::new(actor);
    r.apply(op_at(
        actor,
        1,
        0,
        create_op(parent, NodeId::root(), pos("a")),
    ));
    r.apply(op_at(actor, 2, 0, create_op(s1, parent, pos("a"))));
    r.apply(op_at(actor, 3, 0, create_op(s2, parent, pos("m"))));
    r.apply(op_at(actor, 4, 0, create_op(s3, parent, pos("z"))));

    // Capture positions.
    let pos_s1 = r.tree.position(s1).cloned();
    let pos_s3 = r.tree.position(s3).cloned();

    // Move s2 to a new position.
    r.apply(op_at(actor, 5, 0, move_op(s2, parent, pos("c"))));

    // s1 and s3 untouched.
    assert_eq!(r.tree.position(s1).cloned(), pos_s1);
    assert_eq!(r.tree.position(s3).cloned(), pos_s3);
    assert_eq!(r.tree.position(s2).map(|p| p.as_str()), Some("c"));
}

/// Is there **any** valid position strictly between these bounds?
///
/// `a..=z` non-empty strings are dense enough for a midpoint and not
/// dense enough for a slot under a key whose tail is a single `a`:
/// nothing sorts between `"a"` and `"aa"`, because every non-empty tail
/// starts at `a` or above. Two more shapes have no interior — a tie
/// (`right <= left`), and `left = None` with `right = "a"`, the
/// fractional floor. Every other pair has room: append `"m"` to `left`,
/// or take one byte off `right`.
///
/// Written out here rather than exported from the crate on purpose. It
/// is the *independent* statement of the contract the property below
/// checks `between` against; sharing an implementation would let one bug
/// satisfy both sides.
fn gap_is_empty(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (_, None) => false,
        (None, Some(r)) => r == "a",
        (Some(l), Some(r)) => r <= l || r.strip_prefix(l) == Some("a"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `between` over arbitrary bounds, including the ones it used to
    /// `assert!` on (issue #282). The contract, in one property:
    ///
    /// - it never panics, and always returns a valid position;
    /// - the result always sorts after `left`;
    /// - it sorts before `right` **whenever a key between the bounds
    ///   exists** — when the gap is empty the lower bound wins and the
    ///   upper one is dropped;
    /// - it is a pure function of the bounds, so two devices performing
    ///   the same insert offline converge on the same key.
    #[test]
    fn between_honours_every_bound_that_can_be_honoured(
        left in prop::option::of("[a-z]{1,6}"),
        right in prop::option::of("[a-z]{1,6}"),
        shape in 0u8..6,
    ) {
        // Two independent draws essentially never produce the two shapes
        // whose gap is empty — an exact tie, and `left` plus a single
        // `a` — so the "bound gets dropped" half of the contract would
        // rest entirely on the hand-written unit tests while the
        // property only ever exercised the other half. Bend a share of
        // the cases onto `left` so both halves carry weight.
        let right = match (shape, left.as_deref()) {
            // Tied bounds: no interior at all.
            (0, Some(l)) => Some(l.to_string()),
            // One `a` of tail: still no interior.
            (1, Some(l)) => Some(format!("{l}a")),
            // Two: holds exactly one key, `l + "a"`, and the old
            // bisection walked straight past it.
            (2, Some(l)) => Some(format!("{l}aa")),
            _ => right,
        };
        let l = left.as_deref().map(|s| Fractional::parse(s).unwrap());
        let r = right.as_deref().map(|s| Fractional::parse(s).unwrap());

        let got = Fractional::between(l.as_ref(), r.as_ref());

        prop_assert!(
            Fractional::parse(got.as_str()).is_ok(),
            "{got} is not a valid position"
        );
        prop_assert_eq!(
            got.clone(),
            Fractional::between(l.as_ref(), r.as_ref()),
            "two devices doing this insert must mint the same key"
        );
        if let Some(l) = &l {
            prop_assert!(&got > l, "{got} must sort after left={l}");
        }
        // An empty gap drops the right bound by contract — the caller is
        // the one that verifies and repositions
        // (`outl_actions::tree::position_before`). Every other pair must
        // honour both bounds.
        let empty = gap_is_empty(left.as_deref(), right.as_deref());
        if let Some(r) = r.as_ref().filter(|_| !empty) {
            prop_assert!(
                &got < r,
                "the gap ({left:?}, {right:?}) holds a key, so {got} must be below {r}"
            );
        }
    }

    /// Repeated inserts against a *tied* pair still produce distinct,
    /// ordered keys rather than one key forever — the tie is dropped as a
    /// bound, not carried as one.
    #[test]
    fn inserting_repeatedly_against_a_tie_keeps_climbing(
        key in "[a-z]{1,4}",
        rounds in 1usize..12,
    ) {
        let tied = Fractional::parse(key).unwrap();
        let mut last = tied.clone();
        for _ in 0..rounds {
            let next = Fractional::between(Some(&last), Some(&tied));
            prop_assert!(next > last, "{next} must sort after {last}");
            last = next;
        }
    }
}
