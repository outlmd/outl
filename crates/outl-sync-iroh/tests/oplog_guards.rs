//! Op-log ingest guards: what a peer may write into our `ops/` dir, and what
//! survives a crash.
//!
//! The sibling of `revocation.rs`. That file asks *whether a dialer may be
//! served*; this one asks *what the bytes it sends are allowed to do* once the
//! connection is already authorized — which is a separate question with a
//! separate answer, and skipping it is how an approved-but-buggy (or
//! approved-then-compromised) peer reaches our own actor's file.
//!
//! Three guards live here, found together:
//!
//! 1. **Bucketing.** `write_deduped_batch` buckets on the untrusted `op.actor`.
//!    `outl-core`'s appender refuses a batch carrying a FOREIGN actor's op; the
//!    ingest needed the mirror rule (anything but our OWN) and had none.
//! 2. **Torn-tail heal.** `outl-core`'s appender closes an un-terminated line
//!    before appending. The ingest — a second writer to the same files — did
//!    not, so a kill mid-`write_all` lost the partial op AND the first op of
//!    the next batch.
//! 3. **The cross-process flock.** `ops/.append.lock` had zero coverage:
//!    deleting its `acquire` would have failed nothing.
//!
//! Deny cases outnumber allow cases here on purpose, and the allows are what
//! stop each guard from becoming a sync outage.

use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::LogOp;
use outl_sync_iroh::test_support;

// `common/` is shared by six suites; a helper another suite needs is dead code
// here, and that is not a defect in either file.
#[allow(dead_code)]
mod common;

use common::{now_ms, wait_until, STEP_TIMEOUT};

// ─────────────────────────────────────────────────────────────────────────────
// DENY — op-log ingest
// ─────────────────────────────────────────────────────────────────────────────

/// Build a `LogOp` attributed to `actor`, at `ts_ms`.
fn op_at(actor: ActorId, ts_ms: u64, counter: u64) -> (NodeId, LogOp) {
    let node = NodeId::new();
    (
        node,
        LogOp {
            ts: Hlc::new(ts_ms, counter as u32, actor),
            actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        },
    )
}

/// Read every `(actor, node)` `Create` pair the workspace holds on disk.
fn created_pairs(workspace_root: &Path, reader: ActorId) -> Vec<(String, String)> {
    let storage = JsonlStorage::open(workspace_root.join("ops"), reader).expect("open storage");
    storage
        .all_ops()
        .expect("all ops")
        .into_iter()
        .filter_map(|log| match log.op {
            Op::Create { node, .. } => Some((log.actor.to_string(), node.to_string())),
            _ => None,
        })
        .collect()
}

/// DENY. A peer may not write ops attributed to THIS device's actor.
///
/// `outl_core`'s appender refuses a foreign-actor op; the sync ingest is the
/// mirror rule (anything but our own) and had no guard at all. Two things ride
/// on it — `ops-<local>.jsonl` has a single writer serialized by a lock this
/// path does not hold, and the dedup is `(actor, ts)` and content-blind.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_op_carrying_our_own_actor_id_is_refused_on_ingest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let local = ActorId::new();
    let peer = ActorId::new();

    let (forged_node, forged) = op_at(local, now_ms(), 0);
    let (honest_node, honest) = op_at(peer, now_ms(), 1);

    let applied = test_support::ingest_ops(dir.path(), local, &[forged, honest])
        .await
        .expect("ingest must not error on a forged op — it refuses per-op");

    assert_eq!(
        applied, 1,
        "exactly the peer's op lands; the one claiming our actor is refused"
    );
    let pairs = created_pairs(dir.path(), local);
    assert!(
        pairs
            .iter()
            .any(|(a, n)| a == &peer.to_string() && n == &honest_node.to_string()),
        "the honest op in the same batch must still land — a per-op refusal, \
         not a poisoned batch"
    );
    assert!(
        !pairs.iter().any(|(_, n)| n == &forged_node.to_string()),
        "an op attributed to our own actor reached our own op log"
    );
}

/// DENY. The consequence, spelled out: a forged op pre-claims an HLC slot and
/// the victim's real op is then dropped as a duplicate.
///
/// The dedup key is `(actor, ts)` and ignores content, so this is silent — no
/// error, no conflict, the genuine edit simply is not there afterwards. This
/// test is the reason the guard above is not just tidiness.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forged_op_cannot_pre_claim_our_hlc_slot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let local = ActorId::new();
    let slot_ms = now_ms();

    // The attacker speaks first, claiming `(local, slot)` with its own content.
    let (forged_node, forged) = op_at(local, slot_ms, 7);
    test_support::ingest_ops(dir.path(), local, &[forged])
        .await
        .expect("ingest");

    // The user then authors their real op into that exact slot, through the
    // normal local write path.
    let real_node = NodeId::new();
    {
        let mut storage =
            JsonlStorage::open(dir.path().join("ops"), local).expect("open local storage");
        storage
            .append_op(&LogOp {
                ts: Hlc::new(slot_ms, 7, local),
                actor: local,
                op: Op::Create {
                    node: real_node,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            })
            .expect("the user's own write must succeed");
    }

    let pairs = created_pairs(dir.path(), local);
    assert!(
        pairs.iter().any(|(_, n)| n == &real_node.to_string()),
        "the user's real op is missing — a peer pre-claimed its HLC slot"
    );
    assert!(
        !pairs.iter().any(|(_, n)| n == &forged_node.to_string()),
        "the forged op is on disk under our actor id"
    );
}

/// DENY. A pairing ticket that carries nothing must not decode into a peer.
///
/// The absent-credential case for `PAIRING_ALPN`, the one protocol on this
/// endpoint that does take a credential. `decode_ticket_rejects_garbage`
/// (`src/pairing.rs`) covers malformed input; this covers empty, which a
/// length check written the obvious way lets through as "zero fields, all
/// valid".
#[test]
fn an_absent_ticket_decodes_into_nothing() {
    assert!(outl_sync_iroh::decode_ticket("").is_err());
    assert!(outl_sync_iroh::decode_ticket("   ").is_err());
}

/// ALLOW. The ingest guard refuses OUR actor, not every actor.
///
/// The guard rewritten as `op.actor != some_peer` — or applied to the batch
/// instead of per-op — would pass every deny test above and stop sync dead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ops_from_other_actors_still_land() {
    let dir = tempfile::tempdir().expect("tempdir");
    let local = ActorId::new();
    let peer_b = ActorId::new();
    let peer_c = ActorId::new();

    let (n1, o1) = op_at(peer_b, now_ms(), 0);
    let (n2, o2) = op_at(peer_c, now_ms(), 1);
    // Third-party relay: C's ops reaching us via B is the whole point of the
    // mesh, so the guard must not care which peer handed them over.
    let (n3, o3) = op_at(peer_c, now_ms() + 1, 2);

    let applied = test_support::ingest_ops(dir.path(), local, &[o1, o2, o3])
        .await
        .expect("ingest");
    assert_eq!(applied, 3);

    let nodes: Vec<String> = created_pairs(dir.path(), local)
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    for n in [n1, n2, n3] {
        assert!(nodes.contains(&n.to_string()), "{n} must have landed");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Durability guards found alongside the security work
// ─────────────────────────────────────────────────────────────────────────────

/// A torn tail must not swallow the first op of the next batch.
///
/// A process killed mid-`write_all` leaves a fragment with no terminator.
/// Appending straight after glues the next op onto it, and the reader's
/// glued-line recovery cannot split a TORN prefix from a good op — so the
/// partial op AND the first op of the next batch are both lost. `outl-core`'s
/// appender heals this; the sync ingest, a second writer to the same files,
/// did not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_torn_tail_never_glues_an_incoming_op_onto_a_fragment() {
    use std::io::Write as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let local = ActorId::new();
    let peer = ActorId::new();

    // Land one good op from `peer`, then tear the file mid-line.
    let (_, first) = op_at(peer, now_ms(), 0);
    test_support::ingest_ops(dir.path(), local, &[first])
        .await
        .expect("ingest");
    let path = dir.path().join("ops").join(format!("ops-{peer}.jsonl"));
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open for tearing");
        f.write_all(b"{\"ts\":{\"physical_ms\":1,\"cou")
            .expect("write torn fragment");
    }

    let (next_node, next) = op_at(peer, now_ms() + 1, 1);
    let applied = test_support::ingest_ops(dir.path(), local, &[next])
        .await
        .expect("ingest over a torn tail");
    assert_eq!(applied, 1);

    let nodes: Vec<String> = created_pairs(dir.path(), local)
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert!(
        nodes.contains(&next_node.to_string()),
        "the op appended after a torn tail was glued onto the fragment and lost"
    );
}

/// The cross-process append flock is real and the ingest waits on it.
///
/// `ops/.append.lock` had zero coverage: deleting the `acquire` line failed
/// nothing, while what it prevents (a GUI and an MCP server interleaving whole
/// batches into one `ops-<actor>.jsonl`) was found on a real disk. `flock(2)`
/// locks are per open file description, so a second handle in this process
/// contends exactly like another process would.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ingest_waits_for_the_cross_process_append_flock() {
    use std::time::Duration;

    let dir = tempfile::tempdir().expect("tempdir");
    let local = ActorId::new();
    let peer = ActorId::new();
    let ops_dir = dir.path().join("ops");
    std::fs::create_dir_all(&ops_dir).expect("create ops dir");

    // Hold the lock the way another process would.
    let held = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(ops_dir.join(".append.lock"))
        .expect("open lock file");
    fs2::FileExt::lock_exclusive(&held).expect("take the lock first");

    let (node, op) = op_at(peer, now_ms(), 0);
    let root = dir.path().to_path_buf();
    let ingest = tokio::spawn(async move { test_support::ingest_ops(&root, local, &[op]).await });

    // It must NOT finish while the lock is held elsewhere.
    let early = tokio::time::timeout(Duration::from_millis(750), async {
        // Poll rather than join, so a finished task is observable without
        // consuming the handle.
        wait_until(Duration::from_millis(700), || {
            dir.path()
                .join("ops")
                .join(format!("ops-{peer}.jsonl"))
                .exists()
        })
    })
    .await
    .unwrap_or(false);
    assert!(
        !early,
        "the ingest wrote while another process held ops/.append.lock — the \
         cross-process serialization is gone"
    );

    fs2::FileExt::unlock(&held).expect("release");
    let applied = tokio::time::timeout(STEP_TIMEOUT, ingest)
        .await
        .expect("the ingest must finish once the lock is released")
        .expect("join")
        .expect("ingest");
    assert_eq!(applied, 1);

    let nodes: Vec<String> = created_pairs(dir.path(), local)
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert!(nodes.contains(&node.to_string()));
}
