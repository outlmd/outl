//! What `doctor` does about the **device store** — the one thing it
//! inspects that lives outside the workspace (`<device_dir>/actors/`).
//!
//! Two separate concerns share this file, and both are about a store
//! that is machine-global:
//!
//! - the GC surface (report a dead actor binding, drop it under
//!   `--repair`, refuse to drop one whose workspace may still come back),
//! - and the isolation that keeps this very suite from writing into the
//!   developer's own store while testing it.

use super::*;

/// Build a workspace plus a device store holding one binding for a
/// workspace that is `age` old and no longer on disk.
fn with_stale_binding(age: std::time::Duration) -> (TempDir, PathBuf, TempDir, PathBuf) {
    let (dir, root, _paths) = fresh();
    let store_dir = TempDir::new().expect("device store");
    let actors = store_dir.path().join("actors");
    std::fs::create_dir_all(&actors).unwrap();

    // The parent is present and the workspace is not — the shape a
    // deleted folder leaves, as opposed to an unmounted volume.
    let gone = dir.path().join("deleted-workspace");
    let record = actors.join("01J0000000000000000000000A");
    std::fs::write(
        &record,
        format!(
            "actor=01J0000000000000000000000B\nroot={}\nmachine=01J0000000000000000000000C\n",
            gone.display()
        ),
    )
    .unwrap();
    filetime::set_file_mtime(
        &record,
        filetime::FileTime::from_system_time(std::time::SystemTime::now() - age),
    )
    .unwrap();

    (dir, root, store_dir, record)
}

fn collect_against(path: &Path, do_repair: bool, store: &Path) -> Result<DoctorReport, ApiError> {
    super::collect_internal(
        path,
        true,
        do_repair,
        RepairScope::Guarded,
        &outl_core::device::DeviceStore::at(store),
    )
}

/// The count has to reach the user, and it has to reach them as `info`.
/// A stale binding names a workspace that no longer exists, so there is
/// no sync it can be wrong about — ranking it beside a torn op log is
/// how the loud lines in this report stop being read.
#[test]
fn a_dead_actor_binding_is_reported_without_being_called_a_problem() {
    let (_dir, root, store, _record) = with_stale_binding(outl_core::device::STALE_BINDING_TTL * 2);

    let report = collect_against(&root, false, store.path()).expect("doctor runs");
    let finding = report
        .findings
        .iter()
        .find(|f| f.message.contains("device store"))
        .expect("the stale binding is reported");

    assert!(finding.message.contains('1'), "{}", finding.message);
    assert!(
        matches!(finding.severity, Severity::Info),
        "a tidiness finding must not rank as a warning: {:?}",
        finding.severity
    );
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains("device-store actor binding")),
        "read-only mode must offer what --repair would do: {:?}",
        report.repairable
    );
}

/// `--repair` drops it, and the record is copied into the run's backup
/// generation first — the same contract as every other repair, so the
/// undo is a plain `cp` back.
#[test]
fn repair_drops_a_dead_binding_after_backing_it_up() {
    let (_dir, root, store, record) = with_stale_binding(outl_core::device::STALE_BINDING_TTL * 2);

    let report = collect_against(&root, true, store.path()).expect("doctor --repair runs");
    let repair = report.repair.expect("a repair ran");

    assert!(!record.exists(), "the stale binding should be gone");
    let action = repair
        .actions
        .iter()
        .find(|a| a.kind == "prune_binding")
        .expect("the prune is reported as its own action");
    assert!(action.ok, "{action:?}");

    let backup = PathBuf::from(&repair.backup_dir)
        .join("device-store")
        .join("actors")
        .join(record.file_name().unwrap());
    assert!(
        backup.exists(),
        "the record must be recoverable: {backup:?}"
    );
}

/// **Do not delete.** The TTL is the window in which a user who deleted a
/// workspace folder by mistake can restore it from the trash and keep the
/// actor they have been writing under. Dropping the binding early forks
/// that workspace's actor on its next open — the exact failure the device
/// store exists to prevent.
#[test]
fn repair_leaves_a_recently_deleted_workspaces_binding_alone() {
    let (_dir, root, store, record) =
        with_stale_binding(std::time::Duration::from_secs(60 * 60 * 24));

    let report = collect_against(&root, true, store.path()).expect("doctor --repair runs");

    assert!(record.exists(), "a young binding must survive --repair");
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.message.contains("device store")),
        "nothing to report: {:#?}",
        report.findings
    );
    assert!(
        report
            .repair
            .is_none_or(|r| r.actions.iter().all(|a| a.kind != "prune_binding")),
        "no binding action should have been planned"
    );
}

/// **Do not delete.** `collect_internal` takes a `DeviceStore` so the
/// suite stays off the machine-global one, and `resolve_device_actor` is
/// the call that makes that load-bearing: it *writes*, binding an actor
/// for the workspace when this device has none. It resolved
/// `DeviceStore::open_default()` on its own for one commit, so every
/// doctor test left a record in the shared `.dev-device-store` — one
/// `root=<TempDir>` orphan per run, which is issue #211's leak
/// reintroduced by its own fix — while the binding check judged a
/// different store entirely.
///
/// Asserting the binding lands in *this* store is what catches that:
/// a second `open_default()` anywhere in the pass leaves it empty.
#[test]
fn the_doctor_binds_its_actor_in_the_store_it_was_handed() {
    let (_dir, root, _paths) = fresh();
    let store_dir = TempDir::new().expect("device store");

    collect_against(&root, false, store_dir.path()).expect("doctor runs");

    let bound: Vec<_> = std::fs::read_dir(store_dir.path().join("actors"))
        .expect("the pass must have bound an actor here")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(
        bound.len(),
        1,
        "expected exactly this workspace's binding, got {bound:?}"
    );
    // `actor_for_instance` records the *canonical* root, so compare
    // against that rather than the path the test happened to build.
    let canonical = std::fs::canonicalize(&root).expect("workspace exists");
    let record = std::fs::read_to_string(&bound[0]).unwrap();
    assert!(
        record.contains(&format!("root={}", canonical.display())),
        "the binding must name the workspace under test: {record}"
    );
}
