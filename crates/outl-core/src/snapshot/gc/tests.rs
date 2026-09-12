//! What the snapshot GC removes, and — the half that matters — what it
//! refuses to remove.
//!
//! Every test builds its own [`TempDir`]. Nothing here reads or writes a
//! real workspace, a real device store, or `$HOME` (root `CLAUDE.md`
//! invariant 9's third question; the answer went missing once and made
//! three doctor tests flaky).

use super::*;
use crate::snapshot::{write_to_disk, SCHEMA_VERSION};
use std::collections::{BTreeMap, BTreeSet};
use tempfile::TempDir;

/// A real pre-#207 snapshot: schema 3, bincode. It dies in the postcard
/// parser, so it surfaces as `Decode` and not `SchemaMismatch` — the exact
/// file `~/outl-p2p` has been carrying since June.
const LEGACY_BINCODE_SCHEMA_3: &[u8] =
    include_bytes!("../../../fixtures/legacy-snapshot-schema3.bin");

/// A snapshot whose per-actor cutoff tops out at `high`, so the boot
/// selector ranks it by that number.
fn body_at(actor: ActorId, high: u64) -> SnapshotBody {
    let mut cutoff = BTreeMap::new();
    cutoff.insert(actor, Hlc::new(high, 0, actor));
    SnapshotBody::from_parts(
        actor,
        cutoff,
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("test body encodes")
}

/// Write a decodable `snap-<actor>.bin` whose cutoff tops out at `high`.
fn put(dir: &Path, actor: ActorId, high: u64) {
    write_to_disk(dir, &body_at(actor, high)).expect("write snapshot");
}

fn snap_path(dir: &Path, actor: ActorId) -> PathBuf {
    dir.join(format!("snap-{actor}.bin"))
}

fn dir_in(tmp: &TempDir) -> PathBuf {
    let dir = tmp.path().join(".outl").join("snapshots");
    std::fs::create_dir_all(&dir).expect("create snapshots dir");
    dir
}

fn verdict_of(survey: &Survey, path: &Path) -> SnapshotVerdict {
    survey
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("{} not in the survey", path.display()))
        .verdict
}

// --------------------------------------------------------------- removals

/// The pathological case from `~/outl-p2p`: a snapshot in a foreign
/// encoder that no build of this binary will ever read again. Keeping it
/// buys nothing and costs a full op-log replay on every boot of whichever
/// device owns that actor.
#[test]
fn a_snapshot_this_binary_can_never_decode_is_dropped() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let dead = ActorId::new();
    std::fs::write(snap_path(&dir, dead), LEGACY_BINCODE_SCHEMA_3).unwrap();

    let removed = sweep(&dir, me).expect("sweep");

    assert_eq!(removed, vec![snap_path(&dir, dead)]);
    assert!(!snap_path(&dir, dead).exists());
}

/// The boot selector reads the device's own snapshot before anything
/// else, so a device whose own file is garbage replays the whole log on
/// **every** boot, forever. That one is worth deleting the moment it is
/// proven dead.
#[test]
fn the_devices_own_undecodable_snapshot_is_dropped_on_boot() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    std::fs::write(snap_path(&dir, me), LEGACY_BINCODE_SCHEMA_3).unwrap();

    let err = crate::snapshot::read_best_from_disk(&dir, me).expect_err("own snapshot is garbage");
    assert!(
        matches!(err, SnapshotError::Decode(_) | SnapshotError::HashMismatch),
        "got {err:?}"
    );
    assert!(
        !snap_path(&dir, me).exists(),
        "boot must not keep paying for a file it proved unreadable"
    );
}

/// `read_best_from_disk` ranks candidates by their highest cutoff HLC and
/// returns exactly one. A candidate that is not that winner is never read
/// again by anybody, because cutoffs only move forward.
#[test]
fn a_candidate_the_boot_selector_would_never_choose_is_dropped() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let behind = ActorId::new();
    let ahead = ActorId::new();
    put(&dir, behind, 100);
    put(&dir, ahead, 900);

    let removed = sweep(&dir, me).expect("sweep");

    assert_eq!(removed, vec![snap_path(&dir, behind)]);
    assert!(
        snap_path(&dir, ahead).exists(),
        "the snapshot a future boot would adopt must survive"
    );
}

/// The selector skips a body with an empty cutoff outright (it buys no
/// replay), so such a file has no reader at all.
#[test]
fn a_candidate_with_nothing_to_offer_is_dropped() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let empty = ActorId::new();
    write_to_disk(
        &dir,
        &SnapshotBody::from_parts(
            empty,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap(),
    )
    .unwrap();

    let removed = sweep(&dir, me).expect("sweep");
    assert_eq!(removed, vec![snap_path(&dir, empty)]);
}

/// `write_to_disk` composes in a scratch file and publishes with
/// `rename`. A process killed in between leaves it behind and nothing
/// has ever removed one.
///
/// The name comes from the real producer, not a literal: `write_to_disk`
/// names its scratch per write so two writers for one actor cannot share
/// an inode, and a collector that only knew the old shared name would
/// stop seeing the very files that change made more numerous.
#[test]
fn a_scratch_file_a_killed_writer_abandoned_is_debris() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let orphan = crate::snapshot::scratch_path(&dir.join("snap-01ABC.bin"));
    std::fs::write(&orphan, b"half a snapshot").unwrap();

    assert_eq!(
        stale_tmp(&dir, Duration::ZERO).expect("list"),
        vec![orphan.clone()]
    );
    assert!(prune_tmp(&orphan, Duration::ZERO).expect("prune"));
    assert!(!orphan.exists());
}

/// The scratch name every build before the per-write one used. A
/// workspace can still be carrying one, and it is debris for the same
/// reason — a write that never became a snapshot.
#[test]
fn the_scratch_name_older_builds_left_behind_is_still_debris() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let orphan = dir.join("snap-01ABC.bin.tmp");
    std::fs::write(&orphan, b"half a snapshot").unwrap();

    assert_eq!(
        stale_tmp(&dir, Duration::ZERO).expect("list"),
        vec![orphan.clone()]
    );
    assert!(prune_tmp(&orphan, Duration::ZERO).expect("prune"));
    assert!(!orphan.exists());
}

// --------------------------------------------------------------- refusals

/// This device's own snapshot is what the boot selector reads first, and
/// it is read **unconditionally** — a peer that is further ahead does not
/// displace it. Dropping it for being behind would cost a replay on the
/// one device that cannot recover it from anywhere else.
#[test]
fn the_devices_own_snapshot_is_never_dropped_for_being_behind() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let ahead = ActorId::new();
    put(&dir, me, 100);
    put(&dir, ahead, 900);

    let removed = sweep(&dir, me).expect("sweep");

    assert!(removed.is_empty(), "removed {removed:?}");
    assert!(snap_path(&dir, me).exists());
    assert!(snap_path(&dir, ahead).exists());
}

/// A snapshot claiming a schema this binary has never heard of was
/// written by a **newer** build, on a workspace two builds share. It is
/// somebody's live cache, not garbage, and deleting it starts a
/// delete/rewrite ping-pong between the two binaries.
#[test]
fn a_snapshot_from_a_newer_build_is_kept() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let future = ActorId::new();

    let mut body = body_at(future, 500);
    body.schema_version = SCHEMA_VERSION + 1;
    body.content_hash = crate::snapshot::compute_hash(&body).unwrap();
    std::fs::write(snap_path(&dir, future), body.encode().unwrap()).unwrap();

    let survey = survey(&dir, me).expect("survey");
    assert_eq!(
        verdict_of(&survey, &snap_path(&dir, future)),
        SnapshotVerdict::Inconclusive
    );
    assert!(sweep(&dir, me).expect("sweep").is_empty());
    assert!(snap_path(&dir, future).exists());
}

/// "I could not read it" is not "I read it and it is garbage." A
/// permission error, an `EIO`, an iCloud placeholder that has not
/// materialized — none of those say anything about the bytes, and there
/// is a `remove_file` on the other side of the verdict.
#[test]
fn a_snapshot_that_could_not_be_read_is_kept() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let unreadable = ActorId::new();
    // A directory named like a snapshot reads back as an I/O error on
    // every platform, with no unsafe permission games.
    let path = snap_path(&dir, unreadable);
    std::fs::create_dir(&path).unwrap();

    let survey = survey(&dir, me).expect("survey");
    assert_eq!(verdict_of(&survey, &path), SnapshotVerdict::Inconclusive);
    assert!(sweep(&dir, me).expect("sweep").is_empty());
    assert!(path.exists());
}

/// Listing and pruning are two passes, and a peer pull or a co-resident
/// process can publish a fresh body into that gap. A verdict computed
/// against bytes that are no longer there is not evidence.
#[test]
fn a_file_replaced_since_the_survey_is_not_pruned() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let behind = ActorId::new();
    let ahead = ActorId::new();
    put(&dir, behind, 100);
    put(&dir, ahead, 900);

    let survey = survey(&dir, me).expect("survey");
    let doomed = survey
        .entries
        .iter()
        .find(|e| e.path == snap_path(&dir, behind))
        .expect("entry")
        .clone();
    assert!(doomed.verdict.is_prunable());

    // Somebody publishes a fresher body over the same name.
    put(&dir, behind, 9_000);

    assert!(!prune(&doomed).expect("prune"), "must refuse the new bytes");
    assert!(snap_path(&dir, behind).exists());
}

/// The guard that keeps this from ever reaching `ops/`, a `.md`, or the
/// device store's `iroh/identity.key`. `parent ==`, not `starts_with`,
/// because `Path::starts_with` compares components without normalising.
#[test]
fn nothing_outside_the_snapshots_directory_is_touched() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let elsewhere = tmp.path().join("snap-01ABC.bin");
    std::fs::write(&elsewhere, b"not ours").unwrap();

    let fabricated = SnapshotEntry {
        path: elsewhere.clone(),
        verdict: SnapshotVerdict::Unusable,
        stamp: stamp_of(&elsewhere),
        dir: dir.clone(),
    };
    assert!(!prune(&fabricated).expect("prune"));
    assert!(elsewhere.exists());

    let fresh_tmp = tmp.path().join("snap-01ABC.bin.tmp");
    std::fs::write(&fresh_tmp, b"not ours either").unwrap();
    assert!(!prune_tmp(&fresh_tmp, Duration::ZERO).expect("prune"));
    assert!(fresh_tmp.exists());
}

/// A scratch file younger than the TTL may well be a write in flight —
/// `write_to_disk` fsyncs a multi-MB body between `create` and `rename`.
#[test]
fn a_scratch_file_that_may_still_be_in_flight_is_kept() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let live = dir.join("snap-01ABC.bin.tmp");
    std::fs::write(&live, b"in flight").unwrap();

    assert!(stale_tmp(&dir, STALE_TMP_TTL).expect("list").is_empty());
    assert!(!prune_tmp(&live, STALE_TMP_TTL).expect("prune"));
    assert!(live.exists());
}

/// The sole readable candidate is the one a future boot depends on, even
/// when this device has no snapshot of its own to compare it against.
#[test]
fn the_only_usable_candidate_is_never_dropped() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let peer = ActorId::new();
    put(&dir, peer, 42);

    assert!(sweep(&dir, me).expect("sweep").is_empty());
    assert!(snap_path(&dir, peer).exists());
}

/// A device that has never written a snapshot is a normal state, not an
/// error, and neither is a workspace whose `.outl/` was never created.
#[test]
fn an_empty_or_absent_directory_is_not_an_error() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    assert!(sweep(&dir, ActorId::new()).expect("empty dir").is_empty());

    let missing = tmp.path().join("nowhere");
    assert!(sweep(&missing, ActorId::new())
        .expect("absent dir")
        .is_empty());
    assert!(survey(&missing, ActorId::new())
        .expect("absent dir")
        .entries
        .is_empty());
}

/// A stranger's file in the snapshots directory is not a snapshot, and
/// the GC's business is snapshots.
#[test]
fn a_file_that_is_not_a_snapshot_is_left_alone() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let stranger = dir.join("README.txt");
    std::fs::write(&stranger, b"hello").unwrap();

    assert!(sweep(&dir, ActorId::new()).expect("sweep").is_empty());
    assert!(stranger.exists());
}

/// Opening a workspace is something `outl doctor` does in its
/// **read-only** mode, and `ops_guard.rs` only restores `ops/`. So the
/// boot selector's scan reports nothing and reclaims nothing: the bulk
/// sweep belongs to processes that already write into this directory.
///
/// The single exception is the device's own provably-dead snapshot,
/// pinned by `the_devices_own_undecodable_snapshot_is_dropped_on_boot`:
/// one file, read end to end and refused, in the slot the selector
/// consults on every boot.
#[test]
fn the_boot_selector_reclaims_nothing_on_a_read_only_open() {
    let tmp = TempDir::new().unwrap();
    let dir = dir_in(&tmp);
    let me = ActorId::new();
    let behind = ActorId::new();
    let ahead = ActorId::new();
    let dead = ActorId::new();
    put(&dir, behind, 100);
    put(&dir, ahead, 900);
    std::fs::write(snap_path(&dir, dead), LEGACY_BINCODE_SCHEMA_3).unwrap();

    let adopted = crate::snapshot::read_best_from_disk(&dir, me).expect("scan");

    assert_eq!(
        adopted.map(|b| b.actor),
        Some(ahead),
        "the winner is adopted"
    );
    assert!(snap_path(&dir, behind).exists(), "a read must not reclaim");
    assert!(
        snap_path(&dir, dead).exists(),
        "nor collect somebody else's"
    );
}
