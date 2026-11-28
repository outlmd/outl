//! A peer is never sent ops of its own actor.
//!
//! A headless node restored from a backup lost some of its own ops while a
//! paired desktop kept them. The desktop saw `below > count` for the node's
//! actor on every exchange and shipped a 20,000-op gap window of the node's
//! own log, which the node refuses by design (it is the only writer of that
//! log), so the loop never ended. The request and response now carry the
//! sender's actor id and the selection skips it.
//!
//! This is the over-the-wire half. The selection itself is pinned in
//! `oplog_select/tests.rs`; here a real initiator and responder exchange.
//! Only the initiator's push is observable from outside (`PushedOps`), so
//! that is the direction asserted. The responder side reads the same field
//! through the same `ops_missing_for` call.

#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::mpsc;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::LogOp;
use outl_sync_iroh::test_support;

use common::{fresh_identity, shared_wid, STEP_TIMEOUT};

/// Ops of `actor` with monotonic HLCs at the current wall clock, so they
/// pass the receiver's ingest sanity gate.
fn build_ops(actor: ActorId, count: u32) -> Vec<LogOp> {
    let base_ms = common::now_ms();
    (0..count)
        .map(|i| LogOp {
            ts: Hlc::new(base_ms + u64::from(i), i, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        })
        .collect()
}

fn append_ops(root: &Path, actor: ActorId, ops: &[LogOp]) {
    let mut storage = JsonlStorage::open(root.join("ops"), actor).expect("open storage");
    storage.append_ops(ops).expect("append ops batch");
}

/// Physical line count of `<root>/ops/ops-<actor>.jsonl`.
fn line_count(root: &Path, actor: ActorId) -> usize {
    let path = root.join("ops").join(format!("ops-{actor}.jsonl"));
    std::fs::read_to_string(path)
        .expect("read ops file")
        .lines()
        .filter(|l| !l.is_empty())
        .count()
}

/// B lacks the first four of its own ops and A holds them. A must push
/// none of B's own actor, and must still push everything else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_short_of_its_own_log_is_not_sent_that_log() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let (actor_a, actor_b, actor_x) = (ActorId::new(), ActorId::new(), ActorId::new());

    let b_own = build_ops(actor_b, 10);
    append_ops(dir_a.path(), actor_b, &b_own);
    // B holds the newest six of its own ops: a gap below its own watermark.
    append_ops(dir_b.path(), actor_b, &b_own[4..]);
    // A third actor only A holds, which has to cross in full.
    let x_ops = build_ops(actor_x, 7);
    append_ops(dir_a.path(), actor_x, &x_ops);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_responder(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let (progress_tx, progress_rx) = mpsc::channel::<outl_actions::SyncProgress>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_with_progress(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
            progress_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    let pushed: u64 = progress_rx
        .try_iter()
        .filter_map(|p| match p {
            outl_actions::SyncProgress::PushedOps { count, .. } => Some(count),
            _ => None,
        })
        .sum();
    assert_eq!(
        pushed,
        x_ops.len() as u64,
        "A pushes the third actor's ops and none of B's own: the old selection \
         would have added B's {}-op gap window, which B refuses",
        b_own.len()
    );
    assert_eq!(
        line_count(dir_a.path(), actor_b),
        b_own.len(),
        "A's copy of B's log is untouched"
    );
    assert_eq!(
        line_count(dir_b.path(), actor_b),
        b_own.len() - 4,
        "B's own log is exactly what B had: nothing crossed to be refused"
    );
    assert_eq!(
        line_count(dir_b.path(), actor_x),
        x_ops.len(),
        "the other actor still lands on B"
    );
}
