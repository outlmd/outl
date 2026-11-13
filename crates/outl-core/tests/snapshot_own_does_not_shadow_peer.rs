//! A device's own snapshot must not shadow a peer's that covers more.
//!
//! The boot selector used to return this device's own snapshot the moment
//! it decoded, on the grounds that it "has the tightest cutoff for its
//! local ops". True, and irrelevant to the question being asked: what
//! decides the cost of a boot is how much of the log the body already
//! holds, and the cutoff answers that per actor.
//!
//! Every actor missing from the cutoff reads as "seen none of your ops",
//! so `ops_since_per_actor` yields that actor's entire history into the
//! delta. A phone that writes a snapshot on its first launch — one actor,
//! today's journal — therefore shadowed the desktop's covering twenty
//! actors, and the delta became the whole op log. The convergence guard
//! then refused that delta (most of it sorts below the phone's own single
//! mark), so the boot full-replayed 722,943 ops and iOS killed the app at
//! 3.45 GB. The snapshot it would have needed was sitting next to the one
//! it preferred.

use std::collections::BTreeMap;

use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::snapshot::{read_best_from_disk, write_to_disk, SnapshotBody};
use tempfile::TempDir;

/// A body whose cutoff names `actors`, each marked at `ms`.
fn body(owner: ActorId, actors: &[ActorId], ms: u64) -> SnapshotBody {
    let mut cutoff = BTreeMap::new();
    for a in actors {
        cutoff.insert(*a, Hlc::new(ms, 0, *a));
    }
    SnapshotBody::from_parts(
        owner,
        cutoff,
        BTreeMap::new(),
        BTreeMap::new(),
        Default::default(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("body encodes")
}

/// The phone's case, end to end: its own snapshot is newer and covers
/// one actor; the desktop's is older and covers twenty. The wide one
/// has to win, or the phone replays the entire log.
#[test]
fn a_wide_peer_snapshot_beats_a_narrow_own_one() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let phone = ActorId::new();
    let desktop = ActorId::new();
    let crowd: Vec<ActorId> = (0..20).map(|_| ActorId::new()).collect();

    // The desktop's: twenty actors, marked yesterday.
    write_to_disk(&dir, &body(desktop, &crowd, 1_000)).unwrap();
    // The phone's own: itself alone, marked today — newer, and nearly
    // empty. This is what the first launch writes.
    write_to_disk(&dir, &body(phone, &[phone], 9_000)).unwrap();

    let chosen = read_best_from_disk(&dir, phone)
        .expect("selector reads")
        .expect("something is adoptable");

    assert_eq!(
        chosen.cutoff.len(),
        crowd.len(),
        "the selector picked a body covering {} actor(s); the phone's own \
         narrow snapshot shadowed the desktop's again",
        chosen.cutoff.len()
    );
}

/// The other direction, so the fix is not "always prefer a peer": once
/// this device's own snapshot covers the same ground, it wins on being
/// the more recent mark.
#[test]
fn an_equally_wide_own_snapshot_still_wins() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let me = ActorId::new();
    let peer = ActorId::new();
    let mut everyone: Vec<ActorId> = (0..8).map(|_| ActorId::new()).collect();
    everyone.push(me);

    write_to_disk(&dir, &body(peer, &everyone, 1_000)).unwrap();
    write_to_disk(&dir, &body(me, &everyone, 9_000)).unwrap();

    let chosen = read_best_from_disk(&dir, me)
        .expect("selector reads")
        .expect("something is adoptable");

    let newest = chosen.cutoff.values().map(|h| h.physical_ms).max().unwrap();
    assert_eq!(
        newest, 9_000,
        "with equal coverage the newer body is the cheaper one to adopt"
    );
}

/// A device with no snapshot of its own still adopts a peer's — the
/// behaviour a freshly paired device depends on, unchanged.
#[test]
fn a_device_with_no_own_snapshot_still_adopts_a_peer() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let fresh = ActorId::new();
    let peer = ActorId::new();
    let crowd: Vec<ActorId> = (0..5).map(|_| ActorId::new()).collect();
    write_to_disk(&dir, &body(peer, &crowd, 1_000)).unwrap();

    let chosen = read_best_from_disk(&dir, fresh)
        .expect("selector reads")
        .expect("the peer body is adoptable");
    assert_eq!(chosen.cutoff.len(), crowd.len());
}

/// The same shadowing, reached through corruption instead of coverage.
///
/// The selector read this device's own snapshot first, purely to catch
/// an unusable one and drop it — and then **returned that error**, so a
/// single unreadable own body sent the boot to a full op-log replay with
/// a healthy peer snapshot sitting in the same directory. On the
/// workspace this was measured against that is a replay of 722,943 ops
/// to avoid adopting 48 MB that was right there.
///
/// A snapshot is a cache. One unreadable entry means skip that entry,
/// never skip the cache.
#[test]
fn an_unreadable_own_snapshot_does_not_hide_a_usable_peer() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(".outl").join("snapshots");
    std::fs::create_dir_all(&dir).unwrap();

    let me = ActorId::new();
    let peer = ActorId::new();
    // The peer covers both actors, so it also out-ranks the own body on
    // coverage — the point here is the *error*, not the ranking, and the
    // own file is written last so it cannot be skipped by accident.
    write_to_disk(&dir, &body(peer, &[me, peer], 500)).unwrap();
    std::fs::write(dir.join(format!("snap-{me}.bin")), b"corrupt, not a body").unwrap();

    let adopted = read_best_from_disk(&dir, me)
        .expect("one bad entry must not fail the read")
        .expect("the healthy peer body is still adoptable");
    assert_eq!(
        adopted.cutoff.len(),
        2,
        "the peer snapshot is what should have been adopted"
    );
}

/// And when nothing in the directory decodes, the caller still learns
/// that the cache was unusable rather than that it was empty — both end
/// in a full replay, but only one of them is worth a log line.
#[test]
fn a_directory_of_corrupt_snapshots_reports_an_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(".outl").join("snapshots");
    std::fs::create_dir_all(&dir).unwrap();

    let me = ActorId::new();
    std::fs::write(dir.join(format!("snap-{me}.bin")), b"junk").unwrap();
    std::fs::write(
        dir.join(format!("snap-{}.bin", ActorId::new())),
        b"also junk",
    )
    .unwrap();

    assert!(read_best_from_disk(&dir, me).is_err());
}
