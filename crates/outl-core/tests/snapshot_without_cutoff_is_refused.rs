//! A snapshot with no per-actor cutoff must never be adopted.
//!
//! `boot_from_snapshot` only adopts a snapshot body when the resident
//! delta is a pure temporal **suffix** of it — every delta op newer than
//! every op already folded into the body. The body is an opaque
//! materialized tree, not a reorderable log, so a delta op sorting at or
//! below a body op would need the CRDT to reorder against ops that exist
//! only inside that tree.
//!
//! That check reads the body's `cutoff`. The guard used to be written
//! `if let Some(max) = body.cutoff.values().max()`, which silently turns
//! **"no evidence"** into **"no objection"**: an empty map yields `None`,
//! the branch is skipped, and the body is adopted unchecked.
//!
//! Two things make this worth a dedicated test rather than a comment.
//!
//! **No honest producer can make one.** `build_snapshot_body` returns
//! `None` rather than an empty cutoff, so the only way to hold such a
//! body is a forged or corrupt peer snapshot. That is what makes
//! refusing it obviously correct rather than a trade-off — it breaks no
//! legitimate case, not even a brand-new empty workspace.
//!
//! **The failure is the maximal one, not a near miss.** Every actor
//! absent from the cutoff reads as "has seen none of your ops", so the
//! delta becomes the *entire* local log, replayed on top of an
//! attacker-chosen `nodes` / `block_text`. It also hits the duplicate
//! `Op::Create` path for every node the forged body seeds, which is the
//! defect [RFC 0263](../../../docs/rfcs/0263-create-is-invertible.md)
//! fixed.
//!
//! The transport has its own refusal (`pull_snapshot_from_peer` will not
//! cache such a body), and that is deliberately **not** a substitute for
//! this one: a snapshot also arrives by file transport and by restored
//! backup, paths the puller never sees. `outl-core` is the authoritative
//! refusal; the transport check only stops a poisoned file being written.

use std::collections::{BTreeMap, BTreeSet};

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::snapshot::{self, SnapshotBody};
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// A workspace root with one real op on disk, plus the snapshots dir.
fn workspace_with_one_op() -> (TempDir, ActorId, NodeId) {
    let dir = TempDir::new().expect("tempdir");
    let ops_dir = dir.path().join("ops");
    std::fs::create_dir_all(&ops_dir).expect("mkdir ops");

    let actor = ActorId::new();
    let node = NodeId::from_seed(b"cutoff-test:", "real");
    let mut storage = JsonlStorage::open(ops_dir, actor).expect("open jsonl");
    storage
        .append_ops(&[LogOp {
            ts: Hlc::new(1_000, 0, actor),
            actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }])
        .expect("seed op");

    (dir, actor, node)
}

/// Write a snapshot whose `cutoff` is empty and whose tree claims a node
/// the op log has never heard of — the shape a forged body would take.
fn write_cutoffless_snapshot(root: &std::path::Path, actor: ActorId) -> NodeId {
    let planted = NodeId::from_seed(b"cutoff-test:", "planted-by-attacker");
    let mut nodes = BTreeMap::new();
    nodes.insert(planted, (NodeId::root(), Fractional::first()));

    let body = SnapshotBody::from_parts(
        actor,
        BTreeMap::new(), // ← the empty cutoff under test
        nodes,
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("build body");

    // Written through the real encoder, so the content hash is valid and
    // the body is rejected on its cutoff rather than on integrity.
    snapshot::write_to_disk(&root.join(".outl").join("snapshots"), &body).expect("write snapshot");
    planted
}

#[test]
fn a_snapshot_with_an_empty_cutoff_is_never_adopted() {
    let (dir, actor, real_node) = workspace_with_one_op();
    let planted = write_cutoffless_snapshot(dir.path(), actor);

    let storage = JsonlStorage::open(dir.path().join("ops"), actor).expect("open jsonl");
    let ws = Workspace::open_with_storage(actor, Box::new(storage), Some(dir.path().to_path_buf()))
        .expect("open workspace");

    assert!(
        !ws.booted_from_snapshot(),
        "a snapshot with no per-actor cutoff must be refused and fall back to a full replay"
    );
    assert!(
        !ws.tree().contains(planted),
        "the forged body's node reached the materialized tree — the cutoff guard was skipped"
    );
    assert!(
        ws.tree().contains(real_node),
        "the full-replay fallback must still produce the real tree"
    );
}

#[test]
fn a_snapshot_with_a_real_cutoff_is_still_adopted() {
    // The control. Without this, the test above would also pass if
    // snapshot boot were broken outright, which would hide a far worse
    // regression behind a green security test.
    let (dir, actor, real_node) = workspace_with_one_op();

    let storage = JsonlStorage::open(dir.path().join("ops"), actor).expect("open jsonl");
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(dir.path().to_path_buf()))
            .expect("open workspace");
    ws.save_snapshot().expect("write a legitimate snapshot");
    drop(ws);

    let storage = JsonlStorage::open(dir.path().join("ops"), actor).expect("reopen jsonl");
    let ws = Workspace::open_with_storage(actor, Box::new(storage), Some(dir.path().to_path_buf()))
        .expect("reopen workspace");

    assert!(
        ws.booted_from_snapshot(),
        "a snapshot with a real cutoff must still be adopted — otherwise the guard above \
         is passing for the wrong reason"
    );
    assert!(ws.tree().contains(real_node));
}
