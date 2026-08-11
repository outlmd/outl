//! Sync saga regression suite (Pilar 2).
//!
//! Every bug hand-found during the sync saga gets a NAMED, permanent test here
//! so it fails red if it ever regresses. Each test name is the bug it guards;
//! the doc comment maps it to the saga checklist item. The pure/deterministic
//! resolution + merge tests live in `src/peers.rs` and `src/engine_membership.rs`
//! `#[cfg(test)]` modules; this file holds the over-the-wire (real QUIC,
//! loopback) regressions that need two endpoints.
//!
//! Shared seed/read/wait helpers come from `common/mod.rs` (read-only here); a
//! couple of saga-specific helpers that don't belong in the shared module live
//! at the bottom of THIS file.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::{LogOp, WorkspaceId};
use outl_sync_iroh::test_support;

mod common;

use common::{
    created_nodes_on_disk, disk_has_all_nodes, fresh_identity, seed_ops, shared_wid, wait_until,
    STEP_TIMEOUT,
};

/// Saga bug #2 — HLC far-future op is dropped on ingest (±24h sanity gate).
///
/// A receives an op stamped > 24h in the future from B. `ingest_received_ops`
/// must SKIP it (log + drop), never write it to disk. B's *valid* ops in the
/// same batch still land, proving the gate filters per-op and doesn't poison the
/// whole batch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn far_future_hlc_op_is_skipped_on_ingest() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // B authors one VALID op (now-stamped) and one POISON op stamped ~48h ahead,
    // both under the same actor so they ride the same per-actor batch to A.
    let valid_node = NodeId::new();
    let future_node = seed_one_op_at(dir_b.path(), actor_b, far_future_ms(), 1);
    // Re-open and append the valid op so B's log holds both.
    {
        let mut storage =
            JsonlStorage::open(dir_b.path().join("ops"), actor_b).expect("open B storage");
        storage
            .append_op(&LogOp {
                ts: Hlc::new(common::now_ms(), 0, actor_b),
                actor: actor_b,
                op: Op::Create {
                    node: valid_node,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            })
            .expect("append valid op");
    }

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // B is the responder; A initiates and pulls B's ops.
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
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    // The valid op must arrive...
    let valid_set: BTreeSet<String> = std::iter::once(valid_node.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &valid_set)
        }),
        "A must receive B's valid (now-stamped) op"
    );

    // ...but the far-future op must NEVER land on A's disk.
    let a_nodes: BTreeSet<String> = created_nodes_on_disk(dir_a.path(), actor_a)
        .into_iter()
        .map(|(_actor, node)| node)
        .collect();
    assert!(
        a_nodes.contains(&valid_node.to_string()),
        "valid op present (sanity)"
    );
    assert!(
        !a_nodes.contains(&future_node.to_string()),
        "an op stamped >24h in the future must be skipped on ingest, never written to disk"
    );
}

/// Saga bug #3 — workspace identity is a stable shared id, NOT the local path.
///
/// `same_workspace_id_yields_same_topic_across_paths` (integration.rs) covers the
/// topic derivation. This is the missing END-TO-END half: two devices at
/// genuinely DIFFERENT local paths but presenting the SAME `WorkspaceId` must
/// reconcile their op logs as one workspace. The temp dirs already differ; the
/// load-bearing fact is that sync keys on the shared id, not the path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn different_paths_same_workspace_id_sync_as_one() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    // Distinct local paths — exactly the desktop `~/outl-p2p` vs mobile `…/outl`
    // mismatch the bug was about.
    assert_ne!(
        dir_a.path(),
        dir_b.path(),
        "the two devices must live at different local paths"
    );

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let a_nodes = seed_ops(dir_a.path(), actor_a, 2);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 3);

    // One shared id for both, regardless of path.
    let wid = WorkspaceId::from_raw("CROSSPATHWORKSPACE0000000000");

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
        wid.clone(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(&ep_a, b_addr, dir_a.path(), &wid, actor_a, a_ready_tx),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    let all_nodes: BTreeSet<String> = a_nodes
        .iter()
        .chain(b_nodes.iter())
        .map(|n| n.to_string())
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &all_nodes)
                && disk_has_all_nodes(dir_b.path(), actor_b, &all_nodes)
        }),
        "two devices at different paths but the same workspace id must converge"
    );
    assert_eq!(
        created_nodes_on_disk(dir_a.path(), actor_a),
        created_nodes_on_disk(dir_b.path(), actor_b),
        "both sides hold the identical merged op set"
    );
}

/// Saga bug #7 — a bidirectional push materializes on BOTH sides AND fires BOTH
/// reload signals.
///
/// `bidirectional_delta_sync` (integration.rs) asserts the merged op SET on both
/// sides. It does NOT assert the `peer_ready_tx` reload signal fired on each
/// side — that signal is what tells the workspace to re-read the log and re-paint
/// the UI. If only the initiator (or only the responder) fired it, one device
/// would silently sit on stale ops. This guards the signal in BOTH directions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidirectional_sync_fires_reload_signal_on_both_sides() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let a_nodes = seed_ops(dir_a.path(), actor_a, 2);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 2);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // Responder side: keep the rx so we can prove the responder fired its signal.
    let (b_ready_tx, b_ready_rx) = mpsc::channel::<()>();
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

    // Initiator side: keep the rx to prove the initiator fired its signal too.
    let (a_ready_tx, a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    // Both logs converge (the SET guarantee).
    let all_nodes: BTreeSet<String> = a_nodes
        .iter()
        .chain(b_nodes.iter())
        .map(|n| n.to_string())
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &all_nodes)
                && disk_has_all_nodes(dir_b.path(), actor_b, &all_nodes)
        }),
        "both sides must converge before we check the reload signals"
    );

    // INITIATOR fired its reload signal (it received B's ops).
    assert!(
        recv_within(&a_ready_rx, STEP_TIMEOUT),
        "initiator must fire peer_ready_tx after pulling the peer's ops"
    );
    // RESPONDER fired its reload signal (it received A's pushed ops).
    assert!(
        recv_within(&b_ready_rx, STEP_TIMEOUT),
        "responder must fire peer_ready_tx after ingesting the pushed ops (regression: only one side signalled)"
    );
}

/// Saga bug #1 (sync side) — concurrent appends to the SAME op-log must NOT glue
/// two ops together (`…}}}{"ts":…`).
///
/// Two initiators sync the SAME workspace into a single responder at the same
/// time. The responder owns ONE `SyncProtocolHandler` (cloned per connection via
/// `Arc`, so its `AppendLock` is shared): the lock must serialize the two inbound
/// batches so the responder's `ops-*.jsonl` files never contain a glued line.
/// Every op on disk must parse cleanly through the production `JsonlStorage`
/// reader, and the responder must hold the full union of both initiators' ops.
///
/// The complementary parser-recovery half (a hand-crafted `}}}{` line still
/// loads both ops) is covered core-side by
/// `outl_core::storage::jsonl::tests::recovers_glued_ops_on_one_line`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_appends_never_glue_ops_on_the_responder() {
    let dir_resp = tempfile::tempdir().expect("responder tempdir");
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_resp = ActorId::new();
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // Two initiators each carry a fat batch of ops so their pushes are large
    // enough to interleave at the write layer if the lock weren't held.
    let a_nodes = seed_ops(dir_a.path(), actor_a, 60);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 60);

    let id_resp = fresh_identity(dir_resp.path(), "resp");
    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // Single responder, single handler → single shared AppendLock across both
    // inbound connections.
    let (resp_ready_tx, _resp_ready_rx) = mpsc::channel::<()>();
    let ep_resp = test_support::bind_sync_endpoint(&id_resp)
        .await
        .expect("bind responder endpoint");
    let resp_addr = ep_resp.addr();
    let _router = test_support::spawn_responder(
        ep_resp,
        dir_resp.path().to_path_buf(),
        shared_wid(),
        actor_resp,
        resp_ready_tx,
        &[id_a.node_id(), id_b.node_id()],
    );

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");

    // Fire BOTH initiators concurrently against the same responder.
    let resp_addr_a = resp_addr.clone();
    let dir_a_path = dir_a.path().to_path_buf();
    let a_task = tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel::<()>();
        tokio::time::timeout(
            STEP_TIMEOUT,
            test_support::run_delta_sync(
                &ep_a,
                resp_addr_a,
                &dir_a_path,
                &shared_wid(),
                actor_a,
                tx,
            ),
        )
        .await
        .expect("A delta sync timed out")
        .expect("A delta sync failed");
    });

    let dir_b_path = dir_b.path().to_path_buf();
    let b_task = tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel::<()>();
        tokio::time::timeout(
            STEP_TIMEOUT,
            test_support::run_delta_sync(&ep_b, resp_addr, &dir_b_path, &shared_wid(), actor_b, tx),
        )
        .await
        .expect("B delta sync timed out")
        .expect("B delta sync failed");
    });

    a_task.await.expect("A task panicked");
    b_task.await.expect("B task panicked");

    // The responder must end up with BOTH initiators' full op sets.
    let union: BTreeSet<String> = a_nodes
        .iter()
        .chain(b_nodes.iter())
        .map(|n| n.to_string())
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_resp.path(), actor_resp, &union)
        }),
        "responder must hold the union of both concurrent pushes"
    );

    // The load-bearing assertion: NO glued line on disk. Scan the raw bytes of
    // every ops file for the `}{` signature, and confirm the production reader
    // parses every op (which it would fail to do on a truncated/garbled line the
    // recovery path couldn't split).
    assert_no_glued_lines(dir_resp.path());
    let parsed: BTreeSet<String> = created_nodes_on_disk(dir_resp.path(), actor_resp)
        .into_iter()
        .map(|(_actor, node)| node)
        .collect();
    assert!(
        union.iter().all(|n| parsed.contains(n)),
        "every concurrently-appended op must parse cleanly off disk (no corruption)"
    );
}

/// Saga bug #9 — ops below a receiver's max-HLC watermark were permanently
/// invisible after an out-of-order delivery (the v1 bare-max vector clock).
///
/// A holds ops 1..10 of actor X; B ingested ONLY op 10 (out of order — a
/// newer op landed ahead of the pending backlog, exactly the production
/// state: 4 timestamp retrocessions in the phone's file on the Mac). Under
/// the v1 clock B's watermark said "I have everything ≤ ts10" and the sender
/// never resent 1..9 — permanently invisible. The v2 `ActorClock` count lets
/// A see `below(10) > count(1)` and fall back to the full actor log; B's
/// ingest dedup absorbs the overlap, so B ends with exactly the 10 ops, no
/// duplicates.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backlog_below_watermark_crosses_after_gap_detected() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_x = ActorId::new();

    let ops = build_ops(actor_x, 10);
    append_ops(dir_a.path(), actor_x, &ops);
    // B holds ONLY the newest op — its max watermark, with a 9-op gap below.
    append_ops(dir_b.path(), actor_x, &ops[9..]);

    sync_a_into_b(dir_a.path(), dir_b.path(), actor_a, actor_b).await;

    let all_nodes = created_node_set(&ops);
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &all_nodes)
        }),
        "B must receive the 9-op backlog below its own watermark (gap detection)"
    );
    let (lines, distinct) = ops_file_census(dir_b.path(), actor_x);
    assert_eq!(distinct, 10, "B holds all 10 distinct ops of actor X");
    assert_eq!(
        lines, 10,
        "the full-log resend must not duplicate B's already-present op"
    );
}

/// Saga bug #9 (ingest half) — the receiver drops ops already on disk AND
/// duplicates within the same batch.
///
/// A's log for actor X carries a historic duplicated line (op3 twice — the
/// residue of two pre-lock concurrent pulls) plus ops 1..2 that B already
/// holds. The fast path ships op3 twice in one batch; B must apply it once.
/// Neither side may grow duplicate lines out of the exchange.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ingest_dedups_already_present_ops() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_x = ActorId::new();

    let ops = build_ops(actor_x, 3);
    append_ops(dir_a.path(), actor_x, &ops);
    // Historic duplicated append: op3 exists TWICE on A's disk.
    append_ops(dir_a.path(), actor_x, &ops[2..]);
    // B already holds ops 1..2.
    append_ops(dir_b.path(), actor_x, &ops[..2]);

    sync_a_into_b(dir_a.path(), dir_b.path(), actor_a, actor_b).await;

    let all_nodes = created_node_set(&ops);
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &all_nodes)
        }),
        "B must receive the one op it was missing"
    );
    let (b_lines, b_distinct) = ops_file_census(dir_b.path(), actor_x);
    assert_eq!(b_distinct, 3, "B holds all 3 distinct ops of actor X");
    assert_eq!(
        b_lines, 3,
        "the in-batch duplicate of op3 must be applied exactly once"
    );
    // A's file is untouched: B had nothing A lacks, and the reverse pull must
    // not re-append what A already holds (its historic dup stays as-is).
    let (a_lines, a_distinct) = ops_file_census(dir_a.path(), actor_x);
    assert_eq!(a_distinct, 3, "A still holds 3 distinct ops");
    assert_eq!(
        a_lines, 4,
        "A's pre-existing historic dup line is untouched"
    );
}

/// Saga bug #9 (fallback half) — the full-actor resend converges the receiver
/// without duplicating the ops it already had.
///
/// A holds ops 1..10 of actor X; B holds only 5..10 (has the max, missing the
/// 1..4 prefix). B's clock says `(max=ts10, count=6)`; A counts 10 distinct
/// ops ≤ ts10 → gap → resends the FULL actor log. B's dedup keeps its 6 and
/// appends exactly the missing 4: 10 lines total, never 16.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_actor_resend_converges_and_dedups() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_x = ActorId::new();

    let ops = build_ops(actor_x, 10);
    append_ops(dir_a.path(), actor_x, &ops);
    // B holds the newest 6, missing the 4-op prefix below its watermark.
    append_ops(dir_b.path(), actor_x, &ops[4..]);

    sync_a_into_b(dir_a.path(), dir_b.path(), actor_a, actor_b).await;

    let all_nodes = created_node_set(&ops);
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &all_nodes)
        }),
        "B must converge to the full 10-op actor log"
    );
    let (lines, distinct) = ops_file_census(dir_b.path(), actor_x);
    assert_eq!(distinct, 10, "B holds all 10 distinct ops of actor X");
    assert_eq!(
        lines, 10,
        "full-log fallback must dedup the 6 ops B already had (10 lines, not 16)"
    );
}

// ── Saga-specific helpers (kept out of shared `common/` on purpose) ───────────

/// Bind A + B endpoints, mount the production responder on B, and run one
/// initiator `delta_sync` pass A → B. Shared driver for the gap-detection
/// (saga bug #9) tests; each test only differs in how it seeds the two disks
/// and what it asserts afterwards.
async fn sync_a_into_b(dir_a: &Path, dir_b: &Path, actor_a: ActorId, actor_b: ActorId) {
    let id_a = fresh_identity(dir_a, "a");
    let id_b = fresh_identity(dir_b, "b");

    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_responder(
        ep_b,
        dir_b.to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(&ep_a, b_addr, dir_a, &shared_wid(), actor_a, a_ready_tx),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");
}

/// Build `count` `Op::Create` ops authored by `actor` with monotonic HLCs
/// anchored at the current wall clock (passes the ±24h ingest gate). Returned
/// in ts order so a test can plant arbitrary subsets — a backlog, just the
/// watermark op — on different disks.
fn build_ops(actor: ActorId, count: u32) -> Vec<LogOp> {
    let base_ms = common::now_ms();
    (0..count)
        .map(|i| LogOp {
            ts: Hlc::new(base_ms + i as u64, i, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        })
        .collect()
}

/// Append pre-built ops verbatim into `<root>/ops/ops-<actor>.jsonl` via the
/// production `JsonlStorage`. Appending the same op twice plants a historic
/// duplicated line, exactly like a pre-lock concurrent pull did.
fn append_ops(root: &Path, actor: ActorId, ops: &[LogOp]) {
    let mut storage = JsonlStorage::open(root.join("ops"), actor).expect("open storage");
    // One batched fsync for the lot (`Storage::append_ops`, issue #192); calling
    // this helper twice still plants a historic duplicated line, since the batch
    // append does no dedup of its own.
    storage.append_ops(ops).expect("append ops batch");
}

/// Node-id set of the `Op::Create` ops in `ops` (for `disk_has_all_nodes`).
fn created_node_set(ops: &[LogOp]) -> BTreeSet<String> {
    ops.iter()
        .map(|log| match log.op {
            Op::Create { node, .. } => node.to_string(),
            _ => unreachable!("build_ops only mints Create ops"),
        })
        .collect()
}

/// Raw census of `<root>/ops/ops-<actor>.jsonl`: `(physical line count,
/// distinct HLC count)`. The PHYSICAL line count is the dedup assertion —
/// reading through `all_ops` would mask a duplicated append, since the op log
/// dedups by op id on apply.
fn ops_file_census(root: &Path, actor: ActorId) -> (usize, usize) {
    let path = root.join("ops").join(format!("ops-{actor}.jsonl"));
    let text = std::fs::read_to_string(&path).expect("read ops file");
    let mut lines = 0usize;
    let mut distinct: BTreeSet<Hlc> = BTreeSet::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        lines += 1;
        let op: LogOp = serde_json::from_str(line).expect("parse op line");
        distinct.insert(op.ts);
    }
    (lines, distinct.len())
}

/// A wall-clock ms timestamp comfortably past the receiver's 24h sanity gate
/// (~48h ahead) so the op is unambiguously "far future".
fn far_future_ms() -> u64 {
    common::now_ms() + 48 * 60 * 60 * 1000
}

/// Append a single `Op::Create` op stamped at `physical_ms` (with `logical`
/// counter) under `actor`, returning the created node id. Used to plant a
/// far-future op the HLC gate must drop.
fn seed_one_op_at(workspace_root: &Path, actor: ActorId, physical_ms: u64, logical: u32) -> NodeId {
    let mut storage = JsonlStorage::open(workspace_root.join("ops"), actor).expect("open storage");
    let node = NodeId::new();
    storage
        .append_op(&LogOp {
            ts: Hlc::new(physical_ms, logical, actor),
            actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        })
        .expect("append op");
    node
}

/// Block on an `mpsc` reload-signal receiver until at least one unit arrives or
/// `timeout` elapses. Returns true if the signal fired.
fn recv_within(rx: &mpsc::Receiver<()>, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            Ok(()) => return true,
            Err(mpsc::TryRecvError::Disconnected) => return false,
            Err(mpsc::TryRecvError::Empty) => {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Fail if any `ops-*.jsonl` file under `<root>/ops` contains a glued-JSON
/// signature (`}{` on a single line) — the op-log corruption the append lock
/// exists to prevent.
fn assert_no_glued_lines(workspace_root: &Path) {
    let ops_dir = workspace_root.join("ops");
    let entries = std::fs::read_dir(&ops_dir).expect("read ops dir");
    let mut checked = 0usize;
    for entry in entries {
        let path = entry.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !name.starts_with("ops-") || !name.ends_with(".jsonl") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read ops file");
        for (lineno, line) in text.lines().enumerate() {
            assert!(
                !line.contains("}{"),
                "glued ops detected in {} line {}: concurrent appends were not serialized",
                path.display(),
                lineno + 1
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "expected at least one ops file to inspect");
}

/// Fix: `delta_sync` must NOT report success when the responder never
/// confirms durable ingest.
///
/// A suspended-iPhone / carrier-NAT-drop peer can complete the exchange
/// (the connection tears down cleanly for the initiator) yet never
/// persist the pushed ops. Before this fix, the initiator discarded the
/// close reason and returned `Ok(())`, logging "catch-up: sync ok" while
/// the peer stayed empty — the desktop→mobile "synced ok but nothing
/// arrived" bug. The responder now signals durable ingest with a
/// `close(0, "done")`; the initiator requires it.
#[tokio::test(flavor = "multi_thread")]
async fn initiator_reports_failure_when_responder_never_confirms_ingest() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let actor_a = ActorId::new();
    seed_ops(dir_a.path(), actor_a, 3);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // B is a half-responder: full exchange, but it closes WITHOUT the
    // "done" durable-ingest sentinel.
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_half_responder(ep_b);

    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");

    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta_sync did not finish within the step timeout");

    assert!(
        result.is_err(),
        "delta_sync must report failure when the responder never confirms \
         durable ingest (no `done` close), not a false success"
    );
}

/// Issue #158 — removing a paired device must actually revoke its sync access.
///
/// The serve side used to accept any peer that presented the right
/// `workspace_id`, never checking whether the connecting device was still an
/// APPROVED peer in `peers.json`. A device removed via `peer remove` (dropped
/// from `peers.json`) kept syncing: it could still pull the full history and
/// push edits. This proves the serve-side authorization check:
///
/// 1. **Known peer accepted** — with A in B's `peers.json`, the sync converges
///    (both hold all ops), so the check never breaks a legitimate paired peer.
/// 2. **Removed peer rejected** — after A is removed from B's `peers.json`, a
///    fresh sync from A is REFUSED (`delta_sync` errors on the `unknown-peer`
///    close) and B ingests NONE of A's newly-authored ops.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removed_peer_is_denied_sync() {
    use outl_sync_iroh::{workspace_peers_path, PeersStore};

    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // A and B each seed their own ops; B is the responder.
    let a_first = seed_ops(dir_a.path(), actor_a, 2);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 1);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    // A is an APPROVED peer of B at first (real paired relationship).
    let _router_b = test_support::spawn_responder(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");

    // (1) Known peer: the sync must succeed and both sides converge.
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr.clone(),
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("an approved peer must sync successfully");

    let converged: BTreeSet<String> = a_first
        .iter()
        .chain(b_nodes.iter())
        .map(|n| n.to_string())
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &converged)
        }),
        "an approved peer's ops must reach the responder"
    );

    // Revoke A: remove it from B's peers.json, exactly like `peer remove` does.
    {
        let path = workspace_peers_path(dir_b.path());
        let mut store = PeersStore::load_or_default(&path).expect("load B peers");
        assert!(
            store
                .remove(&id_a.node_id().to_string())
                .expect("remove A from B peers"),
            "A must have been an approved peer before removal"
        );
        assert!(
            store.list().is_empty(),
            "B's peer list is empty after revoking A"
        );
    }

    // A authors NEW ops after being revoked — these must NOT reach B.
    let a_after_revoke = seed_ops(dir_a.path(), actor_a, 2);

    // (2) Removed peer: the sync must be REJECTED (unknown-peer close → the
    // initiator's durable-ingest confirmation never comes, so it errors).
    //
    // The progress sink is here because a refusal is the one failure a user
    // has to ACT on (re-pair), and it was the one failure that reached them as
    // nothing at all: the responder closes before writing a byte, so the
    // initiator dies on a *read* and never reaches the close-reason check at
    // the end. Meanwhile a phone locking its screen painted the panel red.
    // Exactly backwards. Asserting only `result.is_err()` passes either way,
    // which is how it stayed invisible.
    let (a_ready_tx2, _a_ready_rx2) = mpsc::channel::<()>();
    let (progress_tx, progress_rx) = mpsc::channel::<outl_actions::SyncProgress>();
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_with_progress(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx2,
            progress_tx,
        ),
    )
    .await
    .expect("delta sync did not finish within the step timeout");
    assert!(
        result.is_err(),
        "a revoked peer's sync must be refused, not silently succeed"
    );

    let reported: Vec<outl_actions::SyncProgress> = progress_rx.try_iter().collect();
    let refusal = reported.iter().find_map(|p| match p {
        outl_actions::SyncProgress::Failed { error, .. } => Some(error.clone()),
        _ => None,
    });
    let refusal = refusal.unwrap_or_else(|| {
        panic!("a revoked peer must surface a Failed progress event, got: {reported:?}")
    });
    assert!(
        refusal.contains("not paired"),
        "the reason must name the fix (re-pair), not just say something broke: {refusal}"
    );
    // A refusal is NOT the suspended-peer case. Mixing them would paint the
    // one actionable failure amber and bury it.
    assert!(
        !reported
            .iter()
            .any(|p| matches!(p, outl_actions::SyncProgress::Interrupted { .. })),
        "a refusal must never be reported as a transient interruption: {reported:?}"
    );

    // B must NOT have ingested any of A's post-revocation ops.
    let b_nodes_on_disk: BTreeSet<String> = created_nodes_on_disk(dir_b.path(), actor_b)
        .into_iter()
        .map(|(_actor, node)| node)
        .collect();
    for revoked in &a_after_revoke {
        assert!(
            !b_nodes_on_disk.contains(&revoked.to_string()),
            "B must reject ops from a revoked peer (leaked {revoked})"
        );
    }
}

// ── Phase 2: snapshot sync (peer snapshot transfer over iroh) ─────────────────

/// Write a small but non-trivial materialized snapshot for `actor` into
/// `snapshots_dir` via outl-core's OWN writer — the exact `snap-<actor>.bin` a
/// real device holds. One node + a per-actor cutoff so it is a realistic,
/// adoptable snapshot (not an empty one boot would ignore).
fn write_test_snapshot(snapshots_dir: &Path, actor: ActorId) {
    use std::collections::{BTreeMap, BTreeSet};
    let node = NodeId::new();
    let mut nodes = BTreeMap::new();
    nodes.insert(node, (NodeId::root(), Fractional::first()));
    let mut cutoff = BTreeMap::new();
    cutoff.insert(actor, Hlc::new(common::now_ms(), 0, actor));
    let mut block_text = BTreeMap::new();
    block_text.insert(node, "hello from the snapshot".to_string());
    let body = outl_core::SnapshotBody::from_parts(
        actor,
        cutoff,
        nodes,
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        block_text,
    )
    .expect("build test snapshot body");
    outl_core::snapshot::write_to_disk(snapshots_dir, &body).expect("write test snapshot");
}

/// Phase 2 — a freshly-paired device pulls a peer's materialized snapshot over
/// `SNAPSHOT_ALPN` and ends up with it byte-identical in its own
/// `.outl/snapshots/`, so it can boot from settled state instead of receiving +
/// replaying the full op log. Also asserts the reload signal fires so boot
/// adopts it. Runs against the production endpoint shape (sync + snapshot ALPN
/// on ONE endpoint).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_transfers_from_peer_on_pair() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_joiner = tempfile::tempdir().expect("joiner tempdir");
    let host_actor = ActorId::new();

    // Host holds a real materialized snapshot on disk.
    let host_snap_dir = dir_host.path().join(".outl").join("snapshots");
    write_test_snapshot(&host_snap_dir, host_actor);
    let host_snap_path = host_snap_dir.join(format!("snap-{host_actor}.bin"));
    let host_bytes = std::fs::read(&host_snap_path).expect("read host snapshot");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_joiner = fresh_identity(dir_joiner.path(), "joiner");

    let (host_ready_tx, _host_ready_rx) = mpsc::channel::<()>();
    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router_host = test_support::spawn_sync_and_snapshot_responder(
        ep_host,
        dir_host.path().to_path_buf(),
        shared_wid(),
        host_actor,
        host_ready_tx,
        &[id_joiner.node_id()],
    );

    let ep_joiner = test_support::bind_sync_endpoint(&id_joiner)
        .await
        .expect("bind joiner endpoint");
    let (joiner_ready_tx, joiner_ready_rx) = mpsc::channel::<()>();
    let wrote = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_joiner, host_addr, dir_joiner.path(), joiner_ready_tx),
    )
    .await
    .expect("snapshot pull timed out")
    .expect("snapshot pull failed");
    assert!(wrote, "joiner must receive and write the host's snapshot");

    // The joiner ends up with snap-<host_actor>.bin byte-identical to the host's.
    let joiner_snap_path = dir_joiner
        .path()
        .join(".outl")
        .join("snapshots")
        .join(format!("snap-{host_actor}.bin"));
    assert!(
        wait_until(STEP_TIMEOUT, || joiner_snap_path.exists()),
        "the joiner must end up with the peer's snap-<actor>.bin on disk"
    );
    let joiner_bytes = std::fs::read(&joiner_snap_path).expect("read joiner snapshot");
    assert_eq!(
        joiner_bytes, host_bytes,
        "the pulled snapshot must be byte-identical to the peer's"
    );

    // The reload signal fired so boot/reload adopts the freshly cached snapshot.
    assert!(
        joiner_ready_rx.recv_timeout(STEP_TIMEOUT).is_ok(),
        "a received snapshot must fire the reload signal"
    );
}

/// Phase 2 — pulling from a peer that has NO snapshot is harmless: the peer
/// replies with an empty frame, the joiner writes nothing and does not error,
/// and normal op-sync still works with the snapshot ALPN mounted on the same
/// endpoint.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_pull_absent_is_harmless() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_joiner = tempfile::tempdir().expect("joiner tempdir");
    let host_actor = ActorId::new();
    let joiner_actor = ActorId::new();

    // Host has NO snapshot, but DOES have op-log history to sync.
    let host_nodes = seed_ops(dir_host.path(), host_actor, 4);

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_joiner = fresh_identity(dir_joiner.path(), "joiner");

    let (host_ready_tx, _host_ready_rx) = mpsc::channel::<()>();
    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router_host = test_support::spawn_sync_and_snapshot_responder(
        ep_host,
        dir_host.path().to_path_buf(),
        shared_wid(),
        host_actor,
        host_ready_tx,
        &[id_joiner.node_id()],
    );

    let ep_joiner = test_support::bind_sync_endpoint(&id_joiner)
        .await
        .expect("bind joiner endpoint");

    // Snapshot pull against a peer with no snapshot: Ok(false), no error.
    let (snap_ready_tx, _snap_ready_rx) = mpsc::channel::<()>();
    let wrote = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(
            &ep_joiner,
            host_addr.clone(),
            dir_joiner.path(),
            snap_ready_tx,
        ),
    )
    .await
    .expect("snapshot pull timed out")
    .expect("an absent peer snapshot must not error");
    assert!(!wrote, "no snapshot may be written when the peer has none");

    // Nothing was written into the joiner's snapshots dir.
    let joiner_snap_dir = dir_joiner.path().join(".outl").join("snapshots");
    let empty = std::fs::read_dir(&joiner_snap_dir)
        .map(|mut it| it.next().is_none())
        .unwrap_or(true);
    assert!(
        empty,
        "no snapshot file may be written when the peer has none"
    );

    // Op-sync still works over the same endpoints (snapshot ALPN coexists).
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_joiner,
            host_addr,
            dir_joiner.path(),
            &shared_wid(),
            joiner_actor,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    let expected: BTreeSet<String> = host_nodes.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || disk_has_all_nodes(
            dir_joiner.path(),
            joiner_actor,
            &expected
        )),
        "op-sync must still work with the snapshot ALPN mounted on the same endpoint"
    );
}

// ── Phase 2: asset sync (binary uploaded-file transfer over iroh) ─────────────

/// Asset sync — a device pulls a peer's content-addressed binary assets
/// (uploaded PDFs / images) over `ASSET_ALPN`, receiving only the files it
/// lacks (the filename IS the content hash, so a name it already holds is
/// skipped). Mirrors `snapshot_transfers_from_peer_on_pair` for the multi-blob,
/// manifest-negotiated asset protocol.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn assets_transfer_from_peer() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_joiner = tempfile::tempdir().expect("joiner tempdir");

    // Host holds two content-addressed assets.
    let pdf = b"%PDF-1.7 fake pdf bytes for the asset transfer regression test";
    let png = b"\x89PNG fake image bytes for the asset transfer regression test";
    let pdf_name = test_support::write_test_asset(dir_host.path(), pdf, "pdf");
    let png_name = test_support::write_test_asset(dir_host.path(), png, "png");

    // The joiner ALREADY has the PDF (identical content → identical name), so the
    // diff must skip it and transfer only the PNG.
    let joiner_pdf_name = test_support::write_test_asset(dir_joiner.path(), pdf, "pdf");
    assert_eq!(
        joiner_pdf_name, pdf_name,
        "identical content must yield an identical content-addressed name"
    );

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_joiner = fresh_identity(dir_joiner.path(), "joiner");

    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router_host = test_support::spawn_asset_responder(ep_host, dir_host.path().to_path_buf());

    let ep_joiner = test_support::bind_sync_endpoint(&id_joiner)
        .await
        .expect("bind joiner endpoint");
    let written = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_asset_pull(&ep_joiner, host_addr, dir_joiner.path()),
    )
    .await
    .expect("asset pull timed out")
    .expect("asset pull failed");
    assert_eq!(
        written, 1,
        "only the one asset the joiner lacked (the PNG) should transfer"
    );

    // The joiner now holds the PNG byte-identical, and still holds the PDF.
    let joiner_assets = dir_joiner.path().join("assets");
    let joiner_png = joiner_assets.join(&png_name);
    assert!(
        wait_until(STEP_TIMEOUT, || joiner_png.exists()),
        "the joiner must end up with the peer's PNG asset on disk"
    );
    assert_eq!(
        std::fs::read(&joiner_png).expect("read joiner png"),
        png,
        "the pulled asset must be byte-identical to the peer's"
    );
    assert!(
        joiner_assets.join(&pdf_name).exists(),
        "the pre-existing PDF must be left untouched"
    );
}

/// Asset sync — pulling from a peer with no assets is harmless: the peer replies
/// with an empty manifest, the joiner writes nothing and does not error, and no
/// `assets/` dir is conjured on the joiner.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn asset_pull_from_peer_without_assets_is_harmless() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_joiner = tempfile::tempdir().expect("joiner tempdir");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_joiner = fresh_identity(dir_joiner.path(), "joiner");

    // Host never created an assets/ dir.
    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router_host = test_support::spawn_asset_responder(ep_host, dir_host.path().to_path_buf());

    let ep_joiner = test_support::bind_sync_endpoint(&id_joiner)
        .await
        .expect("bind joiner endpoint");
    let written = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_asset_pull(&ep_joiner, host_addr, dir_joiner.path()),
    )
    .await
    .expect("asset pull timed out")
    .expect("an absent-assets pull must not error");
    assert_eq!(written, 0, "nothing transfers when the peer has no assets");

    // An empty manifest must not conjure an assets dir with content.
    let joiner_assets = dir_joiner.path().join("assets");
    let empty = std::fs::read_dir(&joiner_assets)
        .map(|mut it| it.next().is_none())
        .unwrap_or(true);
    assert!(empty, "an empty manifest writes nothing on the joiner");
}
