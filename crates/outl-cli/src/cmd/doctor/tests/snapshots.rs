//! What the doctor is allowed to do to `.outl/snapshots/`.
//!
//! A snapshot is a **pure cache**: the op log is the source of truth, so
//! the entire cost of a wrong verdict here is one slower boot. That is
//! why `--repair` is allowed to reclaim a snapshot at all — and why the
//! bar for *which* one is still not zero, because `remove_file` does not
//! care that the file was "only" a cache.
//!
//! `outl_core::snapshot::gc` owns the verdict (RFC 0258); everything
//! here pins that the doctor asks it rather than forming its own
//! opinion, and that the three verdicts which mean **keep** are honoured:
//!
//! - `Own` — this device's boot cache, read first on every boot;
//! - `Selected` — the candidate a boot with no own snapshot would adopt;
//! - `Inconclusive` — we could not read it, or a newer build wrote it.
//!   "Could not read" is not "read and proved bad".

use super::*;

/// A device store this test owns, so two `doctor` runs are the same
/// device and `snap-<own actor>.bin` means something.
fn owned_store() -> (tempfile::TempDir, outl_core::device::DeviceStore) {
    let dir = tempfile::TempDir::new().expect("temp device store");
    let store = outl_core::device::DeviceStore::at(dir.path());
    (dir, store)
}

fn doctor(
    path: &Path,
    do_repair: bool,
    store: &outl_core::device::DeviceStore,
) -> Result<DoctorReport, ApiError> {
    collect_with_store(path, do_repair, RepairScope::Guarded, store)
}

/// Three decodable snapshots in one directory: this device's own, plus
/// two copies under other actor names.
///
/// Returns `(own, superseded, selected)`. The two copies are
/// byte-identical, so their cutoff HLCs tie and the boot selector's
/// filename tiebreak decides — the lexicographically larger name is the
/// one it would adopt.
fn three_snapshots(
    root: &Path,
    paths: &Paths,
    store: &outl_core::device::DeviceStore,
) -> (PathBuf, PathBuf, PathBuf) {
    seed_page(root, "notes", &["hello"]);
    let cfg = crate::workspace_layout::read_config(paths).unwrap();
    let mut ws = crate::ws::open(root).expect("open");
    ws.workspace.save_snapshot().expect("write snapshot");
    drop(ws);

    let dir = paths.dot_outl.join("snapshots");
    // The name the doctor will read as *this device's own* — which is
    // the store's actor, not `config.toml`'s. `crate::ws::open` writes
    // under whatever it resolves, so the file `save_snapshot` produced
    // is renamed into place under the doctor's actor.
    let actor = outl_ws::actor::resolve_device_actor(paths, &cfg, store).expect("device actor");
    let own = dir.join(format!("snap-{actor}.bin"));
    if !own.exists() {
        let written = std::fs::read_dir(&dir)
            .expect("snapshots dir")
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "bin"))
            .expect("save_snapshot should have written one");
        std::fs::rename(&written, &own).expect("rename to the doctor's actor");
    }

    let mut names = [ulid::Ulid::new().to_string(), ulid::Ulid::new().to_string()];
    names.sort();
    let loser = dir.join(format!("snap-{}.bin", names[0]));
    let winner = dir.join(format!("snap-{}.bin", names[1]));
    std::fs::copy(&own, &loser).expect("copy snapshot");
    std::fs::copy(&own, &winner).expect("copy snapshot");
    (own, loser, winner)
}

/// "I could not read it" is not "I read it and it is garbage", and
/// `--repair` deletes for real.
///
/// The unreadable entry here is a *directory* wearing the snapshot's
/// name, which fails `fs::read` deterministically on every platform
/// (permission bits do not, when the suite runs as root). What matters
/// is the branch: an I/O error must never reach the deletion list.
#[test]
fn a_snapshot_that_cannot_be_read_is_never_deleted() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let snap = paths
        .dot_outl
        .join("snapshots")
        .join(format!("snap-{}.bin", cfg.workspace.actor_id));
    std::fs::create_dir_all(&snap).unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "could not be read"),
        "an unreadable snapshot must be reported as such, got: {:#?}",
        messages(&report)
    );
    assert!(
        !report
            .repairable
            .iter()
            .any(|r| r.contains(&snap.display().to_string())),
        "a file we never read is not a file we know is dead: {:?}",
        report.repairable
    );

    collect(&root, true).expect("doctor --repair runs");
    assert!(
        snap.exists(),
        "`--repair` must not delete a snapshot it could not read"
    );
}

/// A snapshot that decodes but which the boot selector can never choose
/// again is pure reclaimable cache. `--repair` drops it, and the report
/// has to say plainly that nothing is at stake — a user reading
/// "deleting 3 files" about their notes directory should not have to
/// guess.
#[test]
fn a_superseded_snapshot_is_dropped_and_the_report_says_it_is_cache() {
    let (_dir, root, paths) = fresh();
    let (store_dir, store) = owned_store();
    let (_own, loser, winner) = three_snapshots(&root, &paths, &store);

    let report = doctor(&root, false, &store).expect("doctor runs");
    assert!(
        has(&report, "superseded"),
        "a superseded snapshot must be named, got: {:#?}",
        messages(&report)
    );
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains("superseded") && r.contains(&loser.display().to_string())),
        "the superseded snapshot must be offered for deletion, and by name: {:?}",
        report.repairable
    );

    let repaired = doctor(&root, true, &store).expect("doctor --repair runs");
    let rep = repaired.repair.expect("a repair report");
    assert_eq!(rep.failed, 0, "repair actions: {:#?}", rep.actions);
    assert!(!loser.exists(), "the superseded snapshot must be gone");
    assert!(
        winner.exists(),
        "the candidate the boot selector would adopt must survive"
    );
    assert!(
        Path::new(&rep.backup_dir).exists(),
        "a dropped snapshot must still be recoverable from {}",
        rep.backup_dir
    );
    drop(store_dir);
}

/// The refusal that matters most: the GC ranks by the boot selector's
/// own comparison precisely so it can never drop the file that selector
/// would have picked.
#[test]
fn the_snapshot_the_boot_selector_would_adopt_is_never_dropped() {
    let (_dir, root, paths) = fresh();
    let (_store_dir, store) = owned_store();
    let (_own, _loser, winner) = three_snapshots(&root, &paths, &store);

    doctor(&root, true, &store).expect("doctor --repair runs");

    assert!(
        winner.exists(),
        "the selected candidate must survive repair"
    );
}

/// This device's own snapshot is read first and unconditionally, so
/// being outranked by a peer's is not evidence against it. Dropping it
/// costs this device a full op-log replay on every boot.
#[test]
fn the_devices_own_snapshot_is_never_dropped_for_being_behind() {
    let (_dir, root, paths) = fresh();
    let (_store_dir, store) = owned_store();
    let (own, _loser, _winner) = three_snapshots(&root, &paths, &store);

    doctor(&root, true, &store).expect("doctor --repair runs");

    assert!(
        own.exists(),
        "the device's own snapshot must survive repair: {}",
        own.display()
    );
}

/// The listing is taken **before** the workspace opens, so the question
/// is whether that open can invalidate it. For `doctor` it cannot:
/// `collect_internal` opens with `root: None`, which forces a full
/// replay — the doctor has to judge the op log, not a snapshot's opinion
/// of it — and as a side effect means it never reads the snapshots
/// directory and never triggers `gc::drop_own_if_unusable`.
///
/// Pinned because it is the premise of the paragraph above: the moment
/// somebody gives the doctor a real root "for speed", its own run starts
/// deleting the file its report just promised to delete.
#[test]
fn a_doctor_run_never_boots_from_a_snapshot_so_it_cannot_collect_one() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let (_store_dir, store) = owned_store();
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let actor = outl_ws::actor::resolve_device_actor(&paths, &cfg, &store).expect("device actor");
    let snap_dir = paths.dot_outl.join("snapshots");
    std::fs::create_dir_all(&snap_dir).unwrap();
    let own = snap_dir.join(format!("snap-{actor}.bin"));
    std::fs::write(&own, b"this is not a postcard snapshot").unwrap();

    let report = doctor(&root, false, &store).expect("doctor runs");

    assert!(
        own.exists(),
        "a read-only doctor run must not delete anything, including through the boot GC"
    );
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains(&own.display().to_string())),
        "and the promise it makes must therefore still be true: {:?}",
        report.repairable
    );
}

/// The other half: a snapshot that *is* collected between the scan and
/// the repair — by a GUI opening the workspace, by `outl serve`, by a
/// background writer's sweep — must read as "nothing to do", not as a
/// failed repair. A plan is a listing; `outl_core::snapshot::gc` is the
/// authority, and the repair pass re-asks it.
#[test]
fn a_snapshot_collected_between_the_scan_and_the_repair_is_not_a_failure() {
    let (_dir, root, _paths) = fresh();
    let ws = outl_core::workspace::Workspace::open_in_memory(outl_core::id::ActorId::new())
        .expect("in-memory workspace");
    let actor = outl_core::id::ActorId::new();
    let plan = super::super::Plan {
        drop_snapshots: vec![super::super::repair::SnapshotDrop {
            path: root
                .join(".outl")
                .join("snapshots")
                .join(format!("snap-{actor}.bin")),
            verdict: outl_core::snapshot::gc::SnapshotVerdict::Unusable,
            bytes: 31,
        }],
        ..Default::default()
    };

    let store_dir = tempfile::TempDir::new().expect("temp device store");
    let report = super::super::repair::run(
        &ws,
        &root,
        actor,
        &plan,
        &outl_core::device::DeviceStore::at(store_dir.path()),
    );

    assert_eq!(report.failed, 0, "actions: {:#?}", report.actions);
    let action = report
        .actions
        .iter()
        .find(|a| a.kind == "delete_snapshot")
        .expect("a snapshot action");
    assert!(
        action.detail.contains("no longer prunable") || action.detail.contains("already collected"),
        "the reason must say the evidence expired, not that the repair broke: {}",
        action.detail
    );
}

/// `write_to_disk` composes every snapshot in a `.bin.tmp` and publishes
/// it with `rename`, so a killed process leaves one behind. Nothing ever
/// removed them — the doctor called them "harmless" and walked on, which
/// is how a ~13 MB fragment lives forever.
#[test]
fn a_stale_snapshot_scratch_file_is_collected() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let snap_dir = paths.dot_outl.join("snapshots");
    std::fs::create_dir_all(&snap_dir).unwrap();
    let tmp = snap_dir.join("snap-01KZBM0TFWN8V5B2GX3GGACM9D.bin.tmp");
    std::fs::write(&tmp, b"half a snapshot").unwrap();
    backdate(&tmp, 2);

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains("scratch") && r.contains("snap-")),
        "an abandoned scratch file must be offered for deletion: {:?}",
        report.repairable
    );

    collect(&root, true).expect("doctor --repair runs");
    assert!(!tmp.exists(), "the abandoned scratch file must be gone");
}

/// The mirror refusal: a `.bin.tmp` written moments ago may be a
/// co-resident process fsyncing a 13 MB body right now. Deleting it
/// corrupts a live write.
#[test]
fn a_fresh_snapshot_scratch_file_is_left_alone() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let snap_dir = paths.dot_outl.join("snapshots");
    std::fs::create_dir_all(&snap_dir).unwrap();
    let tmp = snap_dir.join("snap-01KZBM0TFWN8V5B2GX3GGACM9D.bin.tmp");
    std::fs::write(&tmp, b"a write in flight").unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        !report.repairable.iter().any(|r| r.contains("scratch")),
        "an in-flight write must not be offered for deletion: {:?}",
        report.repairable
    );

    collect(&root, true).expect("doctor --repair runs");
    assert!(tmp.exists(), "an in-flight write must survive repair");
}
