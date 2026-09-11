//! The `old_*` fields: **storage and the resident log legitimately disagree**,
//! and the disagreement is confined to exactly those fields.
//!
//! `crates/outl-core/CLAUDE.md` spends a long section on this and
//! `stored_op_matches_the_log.rs` pins it with four deterministic cases. What
//! neither states as a *property* is the shape of the divergence, which is the
//! part a refactor can widen without anybody noticing:
//!
//! - `do_op` fills `old_parent` / `old_position` / `old_value` /
//!   `old_until_ms` from the tree as it passes through, so before it runs they
//!   are the caller's guess (usually `root` / `None`).
//! - `Workspace::apply` persists **what the log recorded**, not what the
//!   caller handed in — so an op written while the log was in order is
//!   correct on disk.
//! - A late op makes `apply_op` undo the tail, insert, and *redo* it. Redo
//!   re-derives `old_*` against the new state (Kleppmann Fig. 4 l.37-40,
//!   §3.4). The resident log follows; the already-written line cannot, because
//!   the log is append-only.
//!
//! So "storage equals the resident log" is false on any workspace that has
//! ever received a late op — which is every synced workspace. The true
//! statement, and the one pinned here, is: **every resident op is in storage
//! under the same timestamp, and the two copies agree on every field that is
//! not an undo record.**
//!
//! ## Why the naive assertion is the dangerous one
//!
//! It is the assertion somebody reaches for when adding a `doctor` check, and
//! it passes on every test workspace built in timestamp order. It fails in
//! production, on exactly the users who sync. The counterpart here —
//! [`a_workspace_fed_in_timestamp_order_has_storage_equal_to_its_log`] — pins
//! the narrower claim that *is* true, so the two together say where the line
//! sits rather than leaving the next author to guess.
//!
//! ## Runtime caveat
//!
//! An audit of a real 217,811-op workspace measured the reorder window at 0
//! for every op: every current path feeds `apply_op` pre-sorted, so the redo
//! branch is real but presently unexercised at runtime. These properties are
//! the net under a planned change to that, not a measurement of a hot path.

mod convergence_gen;

use convergence_gen::{lower, program_strategy, Pools};
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{op_node, LogOp, Op};
use outl_core::workspace::Workspace;
use proptest::prelude::*;
use std::collections::BTreeMap;

/// Strip every field whose only reader is `undo_op`, replacing it with a
/// fixed sentinel.
///
/// What survives is the op's **effect** — the part two copies of one op must
/// agree on no matter how many times a reorder re-derived the undo record.
fn without_undo_fields(op: &Op) -> Op {
    match op.clone() {
        Op::Move {
            node,
            new_parent,
            position,
            ..
        } => Op::Move {
            node,
            new_parent,
            position,
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
        Op::SetProp {
            node, key, value, ..
        } => Op::SetProp {
            node,
            key,
            value,
            old_value: None,
        },
        Op::SetCollapsed { node, value, .. } => Op::SetCollapsed {
            node,
            value,
            old_value: false,
        },
        Op::SnoozeRemind { node, until_ms, .. } => Op::SnoozeRemind {
            node,
            until_ms,
            old_until_ms: None,
        },
        // `Create` and `Edit` carry no undo record on the op itself —
        // `Create`'s lives in `Tree::created_by`, and `Edit` has none.
        other => other,
    }
}

/// Every op in storage, keyed by timestamp, gathered through the public
/// `ops_for_node` path (the same route a history panel or `outl doctor`
/// takes).
fn stored_by_ts(ws: &Workspace) -> BTreeMap<Hlc, LogOp> {
    let mut nodes: Vec<NodeId> = ws.log().iter().filter_map(|l| op_node(&l.op)).collect();
    nodes.sort();
    nodes.dedup();

    let mut out = BTreeMap::new();
    for node in nodes {
        for stored in ws.ops_for_node(node).expect("storage is readable") {
            out.insert(stored.ts, stored);
        }
    }
    out
}

/// Feed `ops` to a fresh in-memory workspace in the given order.
fn workspace_fed(ops: &[LogOp]) -> Workspace {
    let mut ws = Workspace::open_in_memory(ActorId::new()).expect("in-memory workspace");
    for op in ops {
        ws.apply(op.clone()).expect("apply");
    }
    ws
}

proptest! {
    // Lower than the tree-only suites: each case opens a workspace and reads
    // every op back through storage, which is the expensive part.
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// No op is lost on the way to storage: every entry in the resident log
    /// is findable on disk under the **same timestamp**.
    ///
    /// Invariant 5 again, one layer down. The tree-level version
    /// (`no_silent_loss_property.rs`) proves the resident log kept the op; this
    /// proves the persisted log did too, which is the copy that survives a
    /// reboot and reaches a peer.
    #[test]
    fn every_resident_op_is_in_storage_under_the_same_timestamp(
        program in program_strategy(),
        shuffle in any::<bool>(),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        if !shuffle {
            ops.sort_by_key(|o| o.ts);
        }

        let ws = workspace_fed(&ops);
        let stored = stored_by_ts(&ws);

        for resident in ws.log().iter() {
            prop_assert!(
                stored.contains_key(&resident.ts),
                "resident op {:?} never reached storage",
                resident.ts
            );
        }
        prop_assert_eq!(
            stored.len(),
            ws.log().len(),
            "storage and the resident log hold different op counts"
        );
    }

    /// The divergence is **bounded**: storage and the resident log may differ
    /// on `old_parent` / `old_position` / `old_value` / `old_until_ms`, and on
    /// nothing else.
    ///
    /// This is the assertion a `doctor` check should make. A reorder rewriting
    /// `new_parent`, `position`, `value` or the node id in one copy and not
    /// the other would be genuine corruption, and length-based or
    /// timestamp-only checks cannot see it.
    #[test]
    fn storage_and_the_resident_log_differ_only_in_the_undo_only_fields(
        program in program_strategy(),
        shuffle in any::<bool>(),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        if !shuffle {
            ops.sort_by_key(|o| o.ts);
        }

        let ws = workspace_fed(&ops);
        let stored = stored_by_ts(&ws);

        for resident in ws.log().iter() {
            let on_disk = stored.get(&resident.ts).expect("op reached storage");
            prop_assert_eq!(on_disk.actor, resident.actor, "actor differs at {:?}", resident.ts);
            prop_assert_eq!(
                without_undo_fields(&on_disk.op),
                without_undo_fields(&resident.op),
                "storage and the resident log disagree about the *effect* of {:?}, \
                 not just its undo record",
                resident.ts
            );
        }
    }

    /// The narrow claim that IS true: a workspace fed strictly in timestamp
    /// order never reorders, so `do_op` runs once per op and the line written
    /// to disk is the final derivation — storage equals the resident log
    /// **exactly**, `old_*` included.
    ///
    /// Pinned so the boundary is explicit. Without it, the previous property
    /// reads as "they are allowed to differ" and invites someone to stop
    /// persisting the derived fields at all.
    #[test]
    fn a_workspace_fed_in_timestamp_order_has_storage_equal_to_its_log(
        program in program_strategy(),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        ops.sort_by_key(|o| o.ts);

        let ws = workspace_fed(&ops);
        let stored = stored_by_ts(&ws);

        for resident in ws.log().iter() {
            let on_disk = stored.get(&resident.ts).expect("op reached storage");
            prop_assert_eq!(
                on_disk,
                resident,
                "in-order workspace: storage does not match the resident log at {:?}",
                resident.ts
            );
        }
    }

    /// A late op makes the resident log's `old_*` **re-derived**, and that
    /// re-derivation must still be a faithful undo record: the op's effect is
    /// untouched, and every op is still present.
    ///
    /// Constructed so a reorder is guaranteed rather than incidental — the
    /// late op carries the smallest possible timestamp, so `apply_op` has to
    /// undo the entire log and redo it.
    #[test]
    fn a_late_op_re_derives_the_undo_record_without_changing_any_effect(
        program in program_strategy(),
    ) {
        let pools = Pools::new();
        let mut ops = lower(&program, &pools);
        ops.sort_by_key(|o| o.ts);

        let ws_before = workspace_fed(&ops);
        let effects_before: BTreeMap<Hlc, Op> = ws_before
            .log()
            .iter()
            .map(|l| (l.ts, without_undo_fields(&l.op)))
            .collect();

        // The same program, plus an op older than all of it. `lower` starts at
        // physical 0, so a dedicated actor sorting below every pool actor at
        // physical 0 is the earliest slot; if the random ids do not cooperate
        // the op is merely early rather than earliest, and the assertions
        // below still hold.
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
                new_parent: NodeId::root(),
                position: Fractional::parse("m").expect("valid position"),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        };

        let mut ws = workspace_fed(&ops);
        ws.apply(late.clone()).expect("late op");

        // Every pre-existing op is still there, and its effect is unchanged —
        // only the undo record was allowed to move.
        for (ts, effect) in &effects_before {
            let resident = ws
                .log()
                .get_by_ts(ts)
                .expect("an op vanished from the resident log across a reorder");
            prop_assert_eq!(
                &without_undo_fields(&resident.op),
                effect,
                "a reorder changed the *effect* of {:?}, not just its undo record",
                ts
            );
        }
        prop_assert!(ws.log().contains_ts(&late.ts), "the late op itself was dropped");
        prop_assert_eq!(
            ws.log().len(),
            effects_before.len() + 1,
            "the reorder changed the log size"
        );

        // And storage still holds everything, under the same timestamps.
        let stored = stored_by_ts(&ws);
        for resident in ws.log().iter() {
            prop_assert!(
                stored.contains_key(&resident.ts),
                "op {:?} is in the resident log but not in storage after a reorder",
                resident.ts
            );
        }
    }
}
