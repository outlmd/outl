//! `outl compact` — behavioural battery for op-log compaction.
//!
//! Compaction drops ops from `ops/*.jsonl`, which is the source of
//! truth (root `CLAUDE.md` invariant 1). Every test here exists to pin
//! one of the two halves of that bargain:
//!
//! - **What it may drop**: a `Move` that merely restates the placement
//!   its own `Create` already made, immediately before it, with nothing
//!   else in between.
//! - **What it must never drop**: every shape where that `Move` is the
//!   op doing the work — above all the trashed-then-restored block,
//!   where `Create` is an idempotent no-op and the `Move` is the only
//!   thing bringing the block back.
//!
//! The predicate and its soundness argument live in
//! [RFC 0256](../../../docs/rfcs/0256-op-log-compaction.md).

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::log::OpLog;
use outl_core::op::{LogOp, Op};
use outl_core::storage::compact::{apply_compaction, plan_compaction, CompactOptions};
use outl_core::tree::Tree;
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;

// --------------------------------------------------------------------------
// Fixture helpers
// --------------------------------------------------------------------------

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid fractional")
}

fn create(ms: u64, actor: ActorId, node: NodeId, parent: NodeId, position: &str) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Create {
            node,
            parent,
            position: pos(position),
        },
    }
}

fn mv(ms: u64, actor: ActorId, node: NodeId, new_parent: NodeId, position: &str) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Move {
            node,
            new_parent,
            position: pos(position),
            // What a producer writes before `do_op` runs. Compaction must
            // never read these — they are the local undo record, not a
            // description of the op's effect.
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    }
}

fn edit(ms: u64, actor: ActorId, node: NodeId) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Edit {
            node,
            text_op: vec![1, 2, 3],
        },
    }
}

/// Lay out a workspace: `<root>/ops/ops-<actor>.jsonl` per actor.
fn workspace(files: &[(ActorId, Vec<LogOp>)]) -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    let ops = tmp.path().join("ops");
    std::fs::create_dir_all(&ops).expect("mkdir ops");
    std::fs::create_dir_all(tmp.path().join(".outl")).expect("mkdir .outl");
    for (actor, log) in files {
        let body: String = log
            .iter()
            .map(|op| serde_json::to_string(op).expect("serialize") + "\n")
            .collect();
        std::fs::write(ops.join(format!("ops-{actor}.jsonl")), body).expect("write ops file");
    }
    tmp
}

fn read_ops(root: &Path) -> Vec<LogOp> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join("ops")).expect("read ops dir") {
        let path = entry.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !(name.starts_with("ops-") && name.ends_with(".jsonl")) {
            continue;
        }
        for line in std::fs::read_to_string(&path).expect("read").lines() {
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(line).expect("parse op"));
        }
    }
    out.sort_by_key(|op: &LogOp| op.ts);
    out
}

/// Canonical materialization: every op applied through the real CRDT in
/// HLC order. Compaction is only correct if this is byte-identical
/// before and after.
fn materialize(ops: &[LogOp]) -> BTreeMap<String, (String, String)> {
    let mut tree = Tree::new();
    let mut log = OpLog::new();
    for op in ops {
        tree.apply_op(&mut log, op.clone());
    }
    tree.iter_nodes()
        .map(|(n, p, position)| {
            (
                n.to_string(),
                (p.to_string(), position.as_str().to_string()),
            )
        })
        .collect()
}

/// Options with the settling horizon disabled, for the tests that are
/// about the predicate rather than about the horizon.
fn no_horizon() -> CompactOptions {
    CompactOptions { horizon_ms: 0 }
}

// --------------------------------------------------------------------------
// What compaction MAY drop
// --------------------------------------------------------------------------

#[test]
fn drops_a_move_that_only_restates_its_own_create() {
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "m"),
        ],
    )]);

    let before = materialize(&read_ops(tmp.path()));
    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(
        plan.report().ops_dropped,
        1,
        "the redundant Move, and only it"
    );

    apply_compaction(tmp.path(), &plan).expect("apply");
    let after_ops = read_ops(tmp.path());
    assert_eq!(after_ops.len(), 2);
    assert_eq!(materialize(&after_ops), before);
}

#[test]
fn any_op_at_all_between_the_create_and_the_move_blocks_the_drop() {
    // The gap rule is "empty", not "holds nothing that could matter" —
    // even an `Op::Edit`, which cannot touch the tree at all. Measured
    // on a 217,811-op workspace the two rules drop exactly the same
    // 62,209 ops, so the cheaper-to-justify one wins: a populated gap
    // is the only evidence we have that an *undelivered* op could
    // occupy the same HLC range.
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            edit(21, a, block),
            mv(22, a, block, page, "m"),
        ],
    )]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

// --------------------------------------------------------------------------
// What compaction must NEVER drop
// --------------------------------------------------------------------------

#[test]
fn keeps_the_move_that_lifts_a_block_back_out_of_the_trash() {
    // THE case. `Op::Create` is idempotent, so the second Create is a
    // no-op on a node that still exists (under TRASH) and the Move is
    // the only op restoring it. Dropping it deletes the user's block
    // and leaves no trace that it happened.
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(30, a, block, NodeId::trash(), "a"),
            create(40, a, block, page, "m"),
            mv(41, a, block, page, "m"),
        ],
    )]);

    let before = materialize(&read_ops(tmp.path()));
    assert_eq!(
        before.get(&block.to_string()).map(|(p, _)| p.clone()),
        Some(page.to_string()),
        "fixture sanity: the block is back on the page"
    );

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(
        plan.report().ops_dropped,
        0,
        "the restoring Move is load-bearing"
    );
    apply_compaction(tmp.path(), &plan).expect("apply");
    assert_eq!(materialize(&read_ops(tmp.path())), before);
}

#[test]
fn keeps_a_move_that_actually_relocates_the_block() {
    let a = ActorId::new();
    let page = NodeId::new();
    let other = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(11, a, other, NodeId::root(), "t"),
            create(20, a, block, page, "m"),
            mv(21, a, block, other, "m"),
        ],
    )]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

#[test]
fn keeps_a_move_that_only_changes_the_position() {
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "t"),
        ],
    )]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

#[test]
fn keeps_a_pair_a_concurrent_actor_interleaved_with() {
    // A peer's op lands, in HLC order, between the Create and the Move.
    // Even when it names another node, a populated gap is evidence of
    // live concurrent writing in that HLC range — which is exactly the
    // window an *undelivered* op could also occupy.
    let a = ActorId::new();
    let b = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let peer_block = NodeId::new();
    let tmp = workspace(&[
        (
            a,
            vec![
                create(10, a, page, NodeId::root(), "m"),
                create(20, a, block, page, "m"),
                mv(30, a, block, page, "m"),
            ],
        ),
        (b, vec![create(25, b, peer_block, page, "t")]),
    ]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

#[test]
fn keeps_a_pair_a_concurrent_move_of_the_same_node_interleaved_with() {
    // The divergence this predicate exists to prevent, made visible:
    // the peer moves the block away, our Move brings it back. Dropping
    // ours silently hands the peer the win.
    let a = ActorId::new();
    let b = ActorId::new();
    let page = NodeId::new();
    let other = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[
        (
            a,
            vec![
                create(10, a, page, NodeId::root(), "m"),
                create(11, a, other, NodeId::root(), "t"),
                create(20, a, block, page, "m"),
                mv(30, a, block, page, "m"),
            ],
        ),
        (b, vec![mv(25, b, block, other, "m")]),
    ]);

    let before = materialize(&read_ops(tmp.path()));
    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
    apply_compaction(tmp.path(), &plan).expect("apply");
    assert_eq!(materialize(&read_ops(tmp.path())), before);
}

#[test]
fn keeps_a_page_root_pair() {
    // A page root's id is `NodeId::from_slug`, so a device that has never
    // heard of this workspace's ops can still name it and `Create` it at
    // any HLC. The causal argument that closes the gap for a random ULID
    // does not hold here at all.
    let a = ActorId::new();
    let page = NodeId::from_slug("ideas");
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            mv(11, a, page, NodeId::root(), "m"),
        ],
    )]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

#[test]
fn keeps_a_pair_whose_node_is_created_twice_anywhere_in_the_log() {
    // Two `Create`s for one id means the id is derivable rather than
    // minted — a template result block, a page root, a future family we
    // have not met. A second device can then mint it with no causal
    // contact, at an HLC below ours, and our Move stops being inert.
    let a = ActorId::new();
    let b = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[
        (
            a,
            vec![
                create(10, a, page, NodeId::root(), "m"),
                create(20, a, block, page, "m"),
                mv(21, a, block, page, "m"),
            ],
        ),
        (b, vec![create(90, b, block, page, "m")]),
    ]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

#[test]
fn keeps_a_move_whose_create_was_written_by_another_actor() {
    let a = ActorId::new();
    let b = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[
        (
            a,
            vec![
                create(10, a, page, NodeId::root(), "m"),
                create(20, a, block, page, "m"),
            ],
        ),
        (b, vec![mv(21, b, block, page, "m")]),
    ]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 0);
}

#[test]
fn keeps_history_inside_the_settling_horizon() {
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let day = 24 * 60 * 60 * 1000u64;
    let tmp = workspace(&[(
        a,
        vec![
            create(100 * day, a, page, NodeId::root(), "m"),
            create(100 * day + 10, a, block, page, "m"),
            mv(100 * day + 11, a, block, page, "m"),
        ],
    )]);

    let plan = plan_compaction(
        tmp.path(),
        &CompactOptions {
            horizon_ms: 30 * day,
        },
    )
    .expect("plan");
    assert_eq!(
        plan.report().ops_dropped,
        0,
        "the pair is newer than the horizon measured from the newest op"
    );
}

// --------------------------------------------------------------------------
// Writing to disk
// --------------------------------------------------------------------------

#[test]
fn planning_alone_never_touches_the_op_log() {
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "m"),
        ],
    )]);
    let path = tmp.path().join("ops").join(format!("ops-{a}.jsonl"));
    let before = std::fs::read(&path).expect("read");

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 1);
    assert_eq!(std::fs::read(&path).expect("read"), before);
}

#[test]
fn apply_backs_up_every_file_it_rewrites() {
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "m"),
        ],
    )]);
    let path = tmp.path().join("ops").join(format!("ops-{a}.jsonl"));
    let original = std::fs::read(&path).expect("read");

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    let report = apply_compaction(tmp.path(), &plan).expect("apply");

    let backup = report.backup_dir.as_ref().expect("a backup was taken");
    let copied = backup.join(format!("ops-{a}.jsonl"));
    assert_eq!(
        std::fs::read(&copied).expect("backup readable"),
        original,
        "the backup must be the pre-compaction bytes, verbatim"
    );
}

#[test]
fn apply_drops_the_offset_index_sidecars_it_invalidated() {
    // Every offset in `.ops-<actor>.idx` / `.nodes.idx` is a byte offset
    // into the file compaction just rewrote. A stale index is silent
    // corruption on the index-driven cold reads, so the sidecars go.
    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "m"),
        ],
    )]);
    let ops = tmp.path().join("ops");
    let idx = ops.join(format!(".ops-{a}.idx"));
    let nodes_idx = ops.join(format!(".ops-{a}.nodes.idx"));
    std::fs::write(&idx, "{}\n").expect("write idx");
    std::fs::write(&nodes_idx, "{}\n").expect("write nodes idx");

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    apply_compaction(tmp.path(), &plan).expect("apply");

    assert!(!idx.exists(), "offset index must not survive a rewrite");
    assert!(!nodes_idx.exists(), "node index must not survive a rewrite");
}

#[test]
fn refuses_to_run_while_another_process_holds_the_workspace() {
    use outl_core::lock::WorkspaceLock;

    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "m"),
        ],
    )]);
    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");

    let _held = WorkspaceLock::acquire(tmp.path()).expect("hold the workspace");
    let err = apply_compaction(tmp.path(), &plan).expect_err("must refuse");
    assert!(
        format!("{err}").contains("open"),
        "the error must say the workspace is open elsewhere, got: {err}"
    );
}

#[test]
fn refuses_when_an_actor_write_lock_is_held() {
    use outl_core::lock::ActorWriteLock;

    let a = ActorId::new();
    let page = NodeId::new();
    let block = NodeId::new();
    let tmp = workspace(&[(
        a,
        vec![
            create(10, a, page, NodeId::root(), "m"),
            create(20, a, block, page, "m"),
            mv(21, a, block, page, "m"),
        ],
    )]);
    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");

    let _held = ActorWriteLock::try_acquire(&tmp.path().join("ops"), a).expect("hold the actor");
    let err = apply_compaction(tmp.path(), &plan).expect_err("must refuse");
    assert!(format!("{err}").contains("open"), "got: {err}");
}

#[test]
fn an_empty_plan_rewrites_nothing_and_takes_no_backup() {
    let a = ActorId::new();
    let page = NodeId::new();
    let tmp = workspace(&[(a, vec![create(10, a, page, NodeId::root(), "m")])]);

    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert!(plan.is_empty());
    let report = apply_compaction(tmp.path(), &plan).expect("apply");
    assert!(report.backup_dir.is_none());
    assert!(!tmp.path().join(".outl").join("compact-backup").exists());
}
