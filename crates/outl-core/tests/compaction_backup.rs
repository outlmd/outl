//! `outl compact --apply` keeps one backup directory per run.
//!
//! The backup under `.outl/compact-backup/` is the only rollback point
//! for the lines a run deleted. It used to be named by a second-
//! resolution timestamp, so a second `--apply` inside that second reused
//! the directory and `copy_durable` overwrote the first run's files.
//! The behavioural battery for *what* compaction may drop lives in
//! `compaction.rs`; this file pins only that the backups survive.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::compact::{apply_compaction, plan_compaction, CompactOptions};
use tempfile::TempDir;

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid fractional")
}

fn create(ms: u64, actor: ActorId, node: NodeId, parent: NodeId) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Create {
            node,
            parent,
            position: pos("m"),
        },
    }
}

/// A `Move` restating its own `Create`: the one shape compaction drops.
fn restating_move(ms: u64, actor: ActorId, node: NodeId, new_parent: NodeId) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Move {
            node,
            new_parent,
            position: pos("m"),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    }
}

#[test]
fn two_applies_in_the_same_second_keep_both_backups() {
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let log = [
        create(10, a, page, NodeId::root()),
        create(20, a, block, page),
        restating_move(21, a, block, page),
    ];
    let original: String = log
        .iter()
        .map(|op| serde_json::to_string(op).expect("serialize") + "\n")
        .collect();

    let tmp = TempDir::new().expect("tempdir");
    let ops = tmp.path().join("ops");
    std::fs::create_dir_all(&ops).expect("mkdir ops");
    std::fs::create_dir_all(tmp.path().join(".outl")).expect("mkdir .outl");
    let path = ops.join(format!("ops-{a}.jsonl"));
    std::fs::write(&path, &original).expect("write ops file");
    let opts = CompactOptions { horizon_ms: 0 };

    let plan = plan_compaction(tmp.path(), &opts).expect("plan");
    let first = apply_compaction(tmp.path(), &plan).expect("first apply");
    let first_dir = first.backup_dir.expect("first backup");

    // Put the pre-compaction log back so there is something to drop
    // again, then run a second apply right away.
    std::fs::write(&path, &original).expect("restore");
    let plan = plan_compaction(tmp.path(), &opts).expect("plan again");
    let second = apply_compaction(tmp.path(), &plan).expect("second apply");
    let second_dir = second.backup_dir.expect("second backup");

    assert_ne!(first_dir, second_dir, "each run gets its own directory");
    let backup = format!("ops-{a}.jsonl");
    assert_eq!(
        std::fs::read_to_string(first_dir.join(&backup)).expect("first backup readable"),
        original,
        "the first run's rollback point must survive the second run"
    );
    assert_eq!(
        std::fs::read_to_string(second_dir.join(&backup)).expect("second backup readable"),
        original,
    );
}
