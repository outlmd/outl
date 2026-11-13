//! A boot that **adopts** a snapshot must still refresh it once the delta
//! has grown, or the remedy disarms itself the moment it works.
//!
//! Every caller used to gate the re-persist on `!booted_from_snapshot()`,
//! which reads as "only bother when there was nothing to adopt". But
//! nothing else refreshes a snapshot on a receive-heavy device: the
//! in-band writer in `Workspace::apply` only fires on a *local* mutation,
//! and ops arriving over sync are written straight into
//! `ops-<actor>.jsonl`. So an adopted snapshot was never rewritten, the
//! delta replayed on top of it only ever grew, and every boot paid for all
//! of it.
//!
//! Measured on a real 722,943-op workspace before the fix: the adopted
//! snapshot covered a third of the log and the boot replayed the other
//! 488,651 ops — the desktop's slow open, and the iPhone's 3.45 GB jetsam
//! kill.

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

/// Open the workspace at `root` the way a client does.
fn open(root: &std::path::Path, actor: ActorId) -> Workspace {
    let ops_dir = root.join(".outl").join("ops");
    Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir, actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap()
}

/// Append `n` `Create` ops through `apply`, so they reach storage.
fn add_ops(ws: &mut Workspace, g: &HlcGenerator, n: usize) {
    for _ in 0..n {
        ws.apply(logop(
            g,
            Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
    }
}

/// The headline: two consecutive snapshot-adopting boots, and the second
/// one must not have a bigger delta than the first.
#[test]
fn an_adopted_snapshot_is_refreshed_once_its_delta_passes_the_threshold() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    const THRESHOLD: u32 = 16;

    // Boot 1: a full replay (no snapshot yet) that writes one.
    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    add_ops(&mut ws, &g, 8);
    assert!(
        !ws.booted_from_snapshot(),
        "first boot has nothing to adopt"
    );
    assert!(
        ws.snapshot_refresh_due(),
        "a full replay always re-persists"
    );
    ws.save_snapshot().unwrap();
    ws.wait_for_snapshots();
    drop(ws);

    // Ops arrive the way sync delivers them: appended to the log by
    // another actor, never through this workspace's `apply`.
    let peer = ActorId::new();
    let gp = HlcGenerator::new(peer);
    let mut peer_ws = open(tmp.path(), peer);
    peer_ws.set_snapshot_policy(false, THRESHOLD);
    add_ops(&mut peer_ws, &gp, THRESHOLD as usize * 2);
    drop(peer_ws);

    // Boot 2: adopts the snapshot and carries a delta past the threshold.
    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    assert!(
        ws.booted_from_snapshot(),
        "boot 2 should adopt the snapshot"
    );
    let delta_two = ws.log().len();
    assert!(
        delta_two >= THRESHOLD as usize,
        "setup: boot 2's delta ({delta_two}) must exceed the threshold"
    );
    assert!(
        ws.snapshot_refresh_due(),
        "an adopted snapshot carrying a {delta_two}-op delta must be refreshed"
    );
    ws.save_snapshot().unwrap();
    ws.wait_for_snapshots();
    drop(ws);

    // Boot 3: the refresh above has to have shrunk what gets replayed.
    let ws = open(tmp.path(), actor);
    let delta_three = ws.log().len();
    assert!(
        delta_three < delta_two,
        "boot 3 replayed {delta_three} ops against boot 2's {delta_two} — the \
         snapshot was not refreshed, so the delta grows without bound"
    );
}

/// The other direction, so the fix cannot become "write on every boot".
/// Rewriting a multi-megabyte body on every launch is its own regression.
#[test]
fn an_adopted_snapshot_with_a_small_delta_is_left_alone() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    const THRESHOLD: u32 = 1_000;

    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    add_ops(&mut ws, &g, 4);
    ws.save_snapshot().unwrap();
    ws.wait_for_snapshots();
    drop(ws);

    let peer = ActorId::new();
    let gp = HlcGenerator::new(peer);
    let mut peer_ws = open(tmp.path(), peer);
    peer_ws.set_snapshot_policy(false, THRESHOLD);
    add_ops(&mut peer_ws, &gp, 3);
    drop(peer_ws);

    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    assert!(ws.booted_from_snapshot(), "setup: should adopt");
    assert!(
        !ws.snapshot_refresh_due(),
        "a {}-op delta is already cheap to replay; rewriting the body here \
         would cost more than it saves",
        ws.log().len()
    );
}

/// The setting still switches the whole thing off.
#[test]
fn snapshots_disabled_never_asks_for_a_refresh() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(false, 16);
    add_ops(&mut ws, &g, 64);
    assert!(!ws.snapshot_refresh_due());
}
