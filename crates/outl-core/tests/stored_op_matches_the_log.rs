//! What lands in storage must be the op the tree recorded, not the one
//! the caller handed in.
//!
//! `Tree::do_op` fills `old_parent` / `old_position` / `old_value` on its
//! way through, so those fields are the caller's guess until it runs.
//! `Workspace::apply` used to persist the caller's copy while appending
//! the corrected one to the resident log, which left the file and the
//! in-memory log disagreeing about every `Move` and every `SetProp`.
//!
//! **Convergence never depended on it**, which is why it survived: every
//! path into the resident log runs `do_op`, and `do_op` overwrites the
//! fields before `undo_op` (its only reader, reached only from
//! `apply_op`) can see them. Peer-sync ingest is the reason that claim
//! is about the resident log and not about storage — it writes a line
//! straight to disk, and `do_op` reaches it on the next boot replay. What depended on it is every reader of the log *as data* — a
//! page history, a debugging human, `outl doctor`. On the workspace this
//! was found in, 65,141 of 65,703 stored `Move` ops named `root` as the
//! old parent regardless of where the block actually was, and all 14,191
//! `SetProp` ops carried a null `old_value`.
//!
//! These tests read the ops back through `Storage`, not through the
//! resident log, because the resident log was always right.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

fn logged(hlc: &HlcGenerator, op: Op) -> LogOp {
    let ts = hlc.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// A workspace holding `parent` under the root and `node` under it.
fn workspace_with_a_nested_block() -> (Workspace, HlcGenerator, NodeId, NodeId) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    let parent = NodeId::new();
    let node = NodeId::new();
    ws.apply(logged(
        &hlc,
        Op::Create {
            node: parent,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .expect("create parent");
    ws.apply(logged(
        &hlc,
        Op::Create {
            node,
            parent,
            position: Fractional::first(),
        },
    ))
    .expect("create node");
    (ws, hlc, parent, node)
}

/// The reconcile and import paths build `Op::Move` without reading the
/// tree first, so they pass `root`. What gets stored has to be what the
/// block's parent actually was.
#[test]
fn a_move_is_stored_with_the_parent_the_tree_knew_not_the_callers_guess() {
    let (mut ws, hlc, parent, node) = workspace_with_a_nested_block();

    ws.apply(logged(
        &hlc,
        Op::Move {
            node,
            new_parent: NodeId::trash(),
            position: Fractional::first(),
            // The lie every non-`move_to` caller tells.
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    ))
    .expect("move");

    let stored = ws.ops_for_node(node).expect("stored ops");
    let old = stored
        .iter()
        .find_map(|logged| match &logged.op {
            Op::Move { old_parent, .. } => Some(*old_parent),
            _ => None,
        })
        .expect("a stored Move");
    assert_eq!(
        old, parent,
        "storage kept the caller's `old_parent` instead of the one `do_op` derived"
    );
}

/// 17 of the 18 `Op::SetProp` construction sites in the workspace
/// hardcode `old_value: None`, so this is the common case, not the edge.
#[test]
fn a_property_write_is_stored_with_the_value_it_replaced() {
    let (mut ws, hlc, _parent, node) = workspace_with_a_nested_block();

    for value in ["first", "second"] {
        ws.apply(logged(
            &hlc,
            Op::SetProp {
                node,
                key: "icon".to_string(),
                value: Some(PropValue::Text(value.to_string())),
                old_value: None,
            },
        ))
        .expect("set prop");
    }

    let stored = ws.ops_for_node(node).expect("stored ops");
    let olds: Vec<Option<PropValue>> = stored
        .iter()
        .filter_map(|logged| match &logged.op {
            Op::SetProp { old_value, .. } => Some(old_value.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        olds,
        vec![None, Some(PropValue::Text("first".to_string()))],
        "the second write should record what it replaced"
    );
}

/// For every op applied **in order**, the bytes in storage and the entry
/// in the resident log are the same op. A test per field would keep
/// missing the next one — `SetCollapsed` also carries an `old_value`
/// nobody has read yet.
///
/// "In order" is load-bearing and is not a weaker version of the claim.
/// See [`a_reorder_leaves_storage_holding_the_first_derivation`] for the
/// case where the two legitimately part ways, and why chasing that would
/// be a bug rather than a fix.
#[test]
fn every_stored_op_equals_its_resident_log_entry_when_applied_in_order() {
    let (mut ws, hlc, parent, node) = workspace_with_a_nested_block();

    ws.apply(logged(
        &hlc,
        Op::SetProp {
            node,
            key: "icon".to_string(),
            value: Some(PropValue::Text("book".to_string())),
            old_value: None,
        },
    ))
    .expect("set prop");
    ws.apply(logged(
        &hlc,
        Op::SetCollapsed {
            node: parent,
            value: true,
            old_value: false,
        },
    ))
    .expect("collapse");
    ws.apply(logged(
        &hlc,
        Op::Move {
            node,
            new_parent: NodeId::root(),
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    ))
    .expect("move");

    for resident in ws.log().iter() {
        let stored = ws
            .ops_for_node(match &resident.op {
                Op::Create { node, .. }
                | Op::Move { node, .. }
                | Op::Edit { node, .. }
                | Op::SetProp { node, .. }
                | Op::SetCollapsed { node, .. }
                | Op::SnoozeRemind { node, .. } => *node,
            })
            .expect("stored ops")
            .into_iter()
            .find(|s| s.ts == resident.ts)
            .expect("every resident op is in storage");
        assert_eq!(
            &stored, resident,
            "storage and the resident log disagree about {:?}",
            resident.ts
        );
    }
}

/// A reorder recomputes `old_*` in the **resident** log and leaves the
/// already-written line alone, so storage and the log diverge — by
/// design, and this pins it so nobody "fixes" it.
///
/// Kleppmann Fig. 4 l.37-40: `redo_op (LogMove t _ p m c)` discards the
/// stored `oldp` and rebuilds the record via `do_op`, because §3.4 says
/// it "might have changed due to the effect of the new operation". The
/// log is append-only, so the line persisted at first application cannot
/// follow it there.
///
/// The temptation this guards against is a `doctor` check asserting
/// "storage must equal the resident log". That would fire on every
/// workspace that has ever received a late op, which is every synced
/// workspace.
#[test]
fn a_reorder_leaves_storage_holding_the_first_derivation() {
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor_a).expect("in-memory workspace");
    let parent = NodeId::new();
    let node = NodeId::new();

    let at = |physical: u64, actor: ActorId, op: Op| LogOp {
        ts: outl_core::hlc::Hlc::new(physical, 0, actor),
        actor,
        op,
    };

    ws.apply(at(
        100,
        actor_a,
        Op::Create {
            node: parent,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .expect("create parent");
    ws.apply(at(
        101,
        actor_a,
        Op::Create {
            node,
            parent,
            position: Fractional::first(),
        },
    ))
    .expect("create node");
    // Applied while the node still sits under `parent`, so this is what
    // gets written to disk.
    let late = at(
        300,
        actor_a,
        Op::Move {
            node,
            new_parent: NodeId::root(),
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    );
    let late_ts = late.ts;
    ws.apply(late).expect("move to root");

    // A peer's earlier op arrives afterwards: `apply_op` undoes the 300,
    // applies this, then redoes the 300 against the new state.
    ws.apply(at(
        200,
        actor_b,
        Op::Move {
            node,
            new_parent: NodeId::trash(),
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    ))
    .expect("late move to trash");

    let resident = match &ws.log().get_by_ts(&late_ts).expect("resident 300").op {
        Op::Move { old_parent, .. } => *old_parent,
        other => panic!("expected a Move, got {other:?}"),
    };
    let stored = ws
        .ops_for_node(node)
        .expect("stored ops")
        .into_iter()
        .find(|o| o.ts == late_ts)
        .map(|o| match o.op {
            Op::Move { old_parent, .. } => old_parent,
            other => panic!("expected a Move, got {other:?}"),
        })
        .expect("stored 300");

    assert_eq!(
        resident,
        NodeId::trash(),
        "the redo should have re-derived against the state the late op left"
    );
    assert_eq!(
        stored, parent,
        "storage should still hold the derivation from first application"
    );
    assert_ne!(
        resident, stored,
        "this test exists to pin the divergence; if they now agree, \
         something started rewriting persisted lines"
    );
}
