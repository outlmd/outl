//! The `Op::Create` undo record is rebuilt correctly on a snapshot boot.
//!
//! `Tree::created_by` — which answers "did *this* `Create` insert this
//! node?", the question `undo_op` needs and `Op::Create` has no field for —
//! is **not** materialized state, so it is absent from the snapshot body and
//! `Tree::from_parts` leaves it empty. That is only sound because of a
//! property of the boot paths rather than of the map: every op that reaches
//! the **resident** log passes through `do_op` first, and the resident log
//! is the only thing `apply_op` can ever undo. Ops folded into the snapshot
//! body are below the cutoff, are not in the resident log, and are
//! therefore unreachable by any undo.
//!
//! This test pins the one shape where that reasoning could go wrong: a node
//! created *below* the cutoff, with a duplicate `Create` *above* it in the
//! delta, and a late op that forces exactly that duplicate to be undone.
//! Get it wrong and the node is deleted from a tree whose op log still
//! holds it, on one device only, after a restart — the failure mode is
//! invisible until the next sync.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn hlc(physical_ms: u64, actor: ActorId) -> Hlc {
    Hlc {
        physical_ms,
        logical: 0,
        actor,
    }
}

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid fractional position")
}

#[test]
fn undoing_a_post_snapshot_duplicate_create_keeps_the_snapshotted_node() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ops_dir = root.join("ops");
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // The shape a deterministic page id produces: both devices address the
    // same node because both computed it from the same slug.
    let page = NodeId::from_slug("ideas");

    let mut ws = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    ws.set_snapshot_policy(false, 0);

    // A creates the page, then snapshots. The Create is now *below* the
    // cutoff: it will never appear in a resident log again.
    ws.apply(LogOp {
        ts: hlc(10_000, actor_a),
        actor: actor_a,
        op: Op::Create {
            node: page,
            parent: NodeId::root(),
            position: pos("a"),
        },
    })
    .unwrap();
    ws.save_snapshot().unwrap();
    drop(ws);

    // B — which never saw A's ops — created the same page offline and moved
    // it. Both land in `ops-B.jsonl` and both sit above A's cutoff, so both
    // replay as delta on A's next boot. B's Move is *older* than B's
    // duplicate Create, which is what makes the duplicate undoable later.
    {
        let mut storage_b = JsonlStorage::open(ops_dir.clone(), actor_b).unwrap();
        storage_b
            .append_op(&LogOp {
                ts: hlc(15_000, actor_b),
                actor: actor_b,
                op: Op::Move {
                    node: page,
                    new_parent: NodeId::root(),
                    position: pos("t"),
                    old_parent: NodeId::root(),
                    old_position: Fractional::first(),
                },
            })
            .unwrap();
        storage_b
            .append_op(&LogOp {
                ts: hlc(20_000, actor_b),
                actor: actor_b,
                op: Op::Create {
                    node: page,
                    parent: NodeId::root(),
                    position: pos("u"),
                },
            })
            .unwrap();
    }

    // Boot A from the snapshot + delta. `created_by` starts empty here; the
    // delta's duplicate Create finds the node already present (it came from
    // the snapshot body) and so records nothing.
    let mut ws2 = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    assert!(
        ws2.tree().contains(page),
        "the page must survive the snapshot boot itself"
    );

    // A late op with a ts below the delta's duplicate Create. `apply_op`
    // pops that Create and undoes it — the exact call that used to delete
    // the node A created before the snapshot.
    ws2.apply(LogOp {
        ts: hlc(17_000, actor_a),
        actor: actor_a,
        op: Op::Move {
            node: page,
            new_parent: NodeId::root(),
            position: pos("w"),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    })
    .unwrap();

    assert!(
        ws2.tree().contains(page),
        "undoing a duplicate Create must not delete a node that Create did \
         not create — here the real creator is an op below the snapshot \
         cutoff, which no resident log holds any more"
    );
    assert_eq!(
        ws2.tree().position(page).map(|p| p.as_str()),
        Some("w"),
        "HLC order is Create@10k, Move@15k, Move@17k, Create@20k(no-op), so \
         the last effective placement is the late Move's"
    );
}
