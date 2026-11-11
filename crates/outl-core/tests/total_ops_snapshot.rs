//! Regression for #289: `Workspace::total_ops()` reports the persisted op-log
//! total, not just the in-memory delta left after a snapshot boot.
//!
//! Before the fix, `log().len()` was shown as the op count. After a snapshot
//! boot that is only the post-cutoff delta, which can be orders of magnitude
//! smaller than the persisted total. `total_ops()` must read the storage count
//! so the two boot paths agree.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

#[test]
fn total_ops_matches_after_snapshot_boot_and_full_replay() {
    let tmp = TempDir::new().unwrap();
    let ops_dir = tmp.path().join(".outl").join("ops");
    let snapshots_dir = tmp.path().join(".outl").join("snapshots");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let root = NodeId::root();

    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    // Apply enough ops that a snapshot is meaningful: creates plus edits.
    const N: usize = 64;
    for i in 0..N {
        let n = NodeId::new();
        ws.apply(logop(
            &g,
            Op::Create {
                node: n,
                parent: root,
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let update = ws.build_text_replace_update(n, &format!("block {i}"));
        ws.apply(logop(
            &g,
            Op::Edit {
                node: n,
                text_op: update,
            },
        ))
        .unwrap();
    }

    // The full-replay path keeps every op in memory.
    assert!(ws.log().len() >= N, "full-replay log holds the applied ops");
    let mut expected_total = ws.total_ops().expect("full-replay count is readable");
    assert_eq!(expected_total, ws.log().len());

    ws.save_snapshot().unwrap();

    // Apply a few more ops *after* the snapshot so the post-boot delta is
    // non-zero. This proves `total_ops()` stays stable while `log().len()`
    // reflects only the new delta.
    let post_snapshot_node = NodeId::new();
    ws.apply(logop(
        &g,
        Op::Create {
            node: post_snapshot_node,
            parent: root,
            position: Fractional::first(),
        },
    ))
    .unwrap();
    expected_total += 1;
    assert_eq!(
        ws.total_ops().expect("count still readable after extra op"),
        expected_total
    );

    drop(ws);

    // Snapshot boot: the in-memory log is only the delta above the cutoff,
    // but the persisted total must stay the same.
    let snap = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    assert!(
        snap.booted_from_snapshot(),
        "test setup must have produced a snapshot boot"
    );
    assert!(
        snap.log().len() < expected_total,
        "snapshot boot leaves only the delta in memory"
    );
    assert_eq!(
        snap.total_ops().expect("snapshot-boot count is readable"),
        expected_total,
        "total_ops must include ops folded into the snapshot"
    );
    assert_eq!(
        snap.log().len(),
        1,
        "only the single post-snapshot op should be in memory"
    );
    drop(snap);

    // Full-replay boot (no snapshot) must report the same total.
    std::fs::remove_dir_all(&snapshots_dir).unwrap();
    let replay = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir, actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    assert!(
        !replay.booted_from_snapshot(),
        "snapshot deletion must force a full replay"
    );
    assert_eq!(
        replay.total_ops().expect("full-replay count is readable"),
        expected_total,
        "full-replay total must match snapshot-boot total"
    );
    assert_eq!(
        replay.log().len(),
        expected_total,
        "full-replay keeps every op in memory"
    );
}
