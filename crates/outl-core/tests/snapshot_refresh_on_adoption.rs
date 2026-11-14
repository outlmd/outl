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
use outl_core::hlc::Hlc;
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

/// A peer generator whose clock has already seen `ws`'s newest op.
///
/// This is what a real peer's clock is: it received our ops before
/// writing its own, and `observe` raised it past them. A bare
/// `HlcGenerator::new` is **not** a smaller version of that — it restarts
/// the logical counter at 0, so whenever our ops all landed in one
/// millisecond (the logical counter climbing 0, 1, 2, ... instead of
/// resetting per op) the peer's first op sorts *below* the snapshot body.
/// `workspace/boot.rs`'s convergence guard then correctly refuses to adopt,
/// the boot full-replays, and `booted_from_snapshot` comes back false.
///
/// Which of the two happened was decided by how long `save_snapshot`'s
/// fsync took: macOS pays `F_FULLFSYNC` (~4ms) and gives every op its own
/// millisecond, Linux does not and packs them into one. So the test passed
/// on a developer's machine and failed on CI, on a clock detail that has
/// nothing to do with what it is testing.
fn peer_clock_after(ws: &Workspace, peer: ActorId) -> HlcGenerator {
    let g = HlcGenerator::new(peer);
    if let Some(newest) = ws.log().last() {
        g.seed(newest.ts);
    }
    g
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
    let peer = ActorId::new();
    let gp = peer_clock_after(&ws, peer);
    drop(ws);

    // Ops arrive the way sync delivers them: appended to the log by
    // another actor, never through this workspace's `apply`.
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
    let peer = ActorId::new();
    let gp = peer_clock_after(&ws, peer);
    drop(ws);

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

/// Append ops at explicit timestamps, so a test can shape the clock
/// instead of hoping the machine shapes it the same way twice.
fn add_ops_at(ws: &mut Workspace, actor: ActorId, ms: u64, logicals: std::ops::Range<u32>) {
    for logical in logicals {
        ws.apply(LogOp {
            ts: Hlc::new(ms, logical, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        })
        .unwrap();
    }
}

/// The other half of the clock rule, pinned so a future flake here is
/// fixed in the setup and never by relaxing the guard.
///
/// A peer whose clock never saw ours writes ops that sort **below** the
/// snapshot body. Adopting there would lay a delta that is not a pure
/// temporal suffix over an opaque materialized tree, which can diverge
/// from a full replay (`workspace/boot.rs`'s convergence guard). So the
/// refusal is correct, and `booted_from_snapshot` reporting false is what
/// that refusal looks like — never evidence that the refresh rule broke.
///
/// This is the exact shape the two tests above produced by accident on a
/// machine with a cheap fsync: our ops packed into one millisecond, the
/// peer's logical counter restarting under them.
#[test]
fn a_delta_that_sorts_below_the_body_is_not_adopted() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    const THRESHOLD: u32 = 16;
    let ms = 1_794_662_000_000;

    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    add_ops_at(&mut ws, actor, ms, 0..8);
    ws.save_snapshot().unwrap();
    ws.wait_for_snapshots();
    drop(ws);

    let peer = ActorId::new();
    let mut peer_ws = open(tmp.path(), peer);
    peer_ws.set_snapshot_policy(false, THRESHOLD);
    add_ops_at(&mut peer_ws, peer, ms, 0..THRESHOLD * 2);
    drop(peer_ws);

    let ws = open(tmp.path(), actor);
    assert!(
        !ws.booted_from_snapshot(),
        "a delta holding ops at or below the body's newest HLC must send \
         the boot to a full replay, not be adopted on top of it"
    );
}

/// And the same peer one millisecond later is adopted, so the test above
/// is pinning the ordering rule and not merely "peers are never adopted".
#[test]
fn a_delta_above_the_body_is_adopted() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    const THRESHOLD: u32 = 16;
    let ms = 1_794_662_000_000;

    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    add_ops_at(&mut ws, actor, ms, 0..8);
    ws.save_snapshot().unwrap();
    ws.wait_for_snapshots();
    drop(ws);

    let peer = ActorId::new();
    let mut peer_ws = open(tmp.path(), peer);
    peer_ws.set_snapshot_policy(false, THRESHOLD);
    add_ops_at(&mut peer_ws, peer, ms + 1, 0..THRESHOLD * 2);
    drop(peer_ws);

    let ws = open(tmp.path(), actor);
    assert!(ws.booted_from_snapshot());
}

/// The headline test, with the clock shape that broke it pinned instead
/// of left to the machine.
///
/// `add_ops` through a generator gives each op whatever millisecond the
/// wall clock is in, so on macOS (one op per millisecond, logical always
/// 0) the headline test exercised a different ordering than on CI (eight
/// ops in one millisecond, logical climbing to 7). This runs the CI shape
/// everywhere, so a regression in `peer_clock_after` fails on a laptop
/// rather than only in the Linux job.
#[test]
fn an_adopted_snapshot_survives_a_packed_millisecond() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    const THRESHOLD: u32 = 16;
    let ms = 1_794_662_000_000;

    let mut ws = open(tmp.path(), actor);
    ws.set_snapshot_policy(true, THRESHOLD);
    // The CI shape: every op of ours in ONE millisecond, logical 0..7.
    add_ops_at(&mut ws, actor, ms, 0..8);
    ws.save_snapshot().unwrap();
    ws.wait_for_snapshots();
    let peer = ActorId::new();
    let gp = peer_clock_after(&ws, peer);
    drop(ws);

    let mut peer_ws = open(tmp.path(), peer);
    peer_ws.set_snapshot_policy(false, THRESHOLD);
    add_ops(&mut peer_ws, &gp, THRESHOLD as usize * 2);
    drop(peer_ws);

    let ws = open(tmp.path(), actor);
    assert!(
        ws.booted_from_snapshot(),
        "the seeded peer clock must survive the packed-millisecond shape"
    );
}
