use super::*;
use tempfile::TempDir;

fn store() -> (TempDir, DeviceStore) {
    let dir = TempDir::new().unwrap();
    let store = DeviceStore::at(dir.path());
    (dir, store)
}

/// A workspace directory with a persisted id, so the path-vs-id logic has
/// something real to read back.
fn workspace() -> (TempDir, WorkspaceId) {
    let dir = TempDir::new().unwrap();
    let id = WorkspaceId::read_or_create(dir.path()).unwrap();
    (dir, id)
}

#[test]
fn machine_id_is_stable_across_calls() {
    let (_d, store) = store();
    assert_eq!(store.machine_id().unwrap(), store.machine_id().unwrap());
}

/// Regression, and the reason `machine_id` mints under `O_EXCL` instead
/// of a plain write — plus the reason `create_new_record` publishes its
/// content by `link(2)` rather than filling an `O_EXCL`-created file.
///
/// **Do not delete.** One device store is shared by every process on the
/// machine, and its `machine-id` arbitrates *every* actor binding and
/// every `actor_claimed_by` claim. Two processes racing the first-ever
/// read used to each mint their own id, last writer winning, so the
/// loser's already-written claims all read as foreign afterwards. A
/// forked actor costs one file; a lost claim is durable — the claim lives
/// in the workspace's `config.toml`, so that workspace never adopts its
/// own legacy ops file again.
///
/// Not hypothetical: this made `outl-cli`'s doctor suite fail on every
/// `cargo test --workspace` that started from a cold
/// shared dev device store. `init` stamped the claim, and the `open` right
/// after it read a machine id a concurrent test binary had just
/// overwritten.
///
/// Racing threads here stand in for racing processes: both reach the same
/// two filesystem primitives, and a thread race is deterministic enough
/// to fail loudly if either primitive stops being atomic.
#[test]
fn concurrent_first_opens_converge_on_one_machine_id() {
    let dir = TempDir::new().unwrap();
    let ids: Vec<MachineId> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..16)
            .map(|_| s.spawn(|| DeviceStore::at(dir.path()).machine_id().unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let distinct: std::collections::BTreeSet<_> = ids.iter().map(MachineId::as_str).collect();
    assert_eq!(
        distinct.len(),
        1,
        "16 racing first opens must agree on one machine id, got {distinct:?}"
    );
    assert_eq!(
        DeviceStore::at(dir.path()).machine_id().unwrap().as_str(),
        ids[0].as_str(),
        "and a later uncontended open reads back the id they agreed on"
    );
}

#[test]
fn distinct_stores_get_distinct_machine_ids() {
    let (_a, a) = store();
    let (_b, b) = store();
    assert_ne!(a.machine_id().unwrap(), b.machine_id().unwrap());
}

#[test]
fn workspace_actor_round_trips_and_is_stable() {
    let (_d, store) = store();
    let (ws_dir, ws) = workspace();
    let actor = ActorId::new();
    assert_eq!(
        store.actor_for_instance(&ws, ws_dir.path(), actor).unwrap(),
        actor
    );
    assert_eq!(
        store
            .actor_for_instance(&ws, ws_dir.path(), ActorId::new())
            .unwrap(),
        actor,
        "a bound instance ignores the fallback"
    );
}

#[test]
fn workspace_actors_are_keyed_per_workspace() {
    let (_d, store) = store();
    let (dir_a, a) = workspace();
    let (dir_b, b) = workspace();
    let (actor_a, actor_b) = (ActorId::new(), ActorId::new());
    store.actor_for_instance(&a, dir_a.path(), actor_a).unwrap();
    assert_eq!(
        store.actor_for_instance(&b, dir_b.path(), actor_b).unwrap(),
        actor_b
    );
}

/// `cp -R notes notes-backup` gives two directories one workspace id.
/// Keyed on the id alone they share an actor — and the P2P transport keys
/// its gossip topic on that same id, so the two copies reconcile as one
/// workspace and dedup their genuinely-distinct ops against each other by
/// `ts`.
#[test]
fn a_copied_workspace_directory_gets_its_own_actor() {
    let (_d, store) = store();
    let (original, ws) = workspace();
    let original_actor = ActorId::new();
    store
        .actor_for_instance(&ws, original.path(), original_actor)
        .unwrap();

    // The copy carries `.outl/workspace-id` verbatim.
    let copy = TempDir::new().unwrap();
    ws.write(copy.path()).unwrap();

    let copy_actor = store
        .actor_for_instance(&ws, copy.path(), ActorId::new())
        .unwrap();
    assert_ne!(copy_actor, original_actor, "two live copies must not share");
    // Both stay put across re-opens — no ping-pong over one slot.
    assert_eq!(
        store
            .actor_for_instance(&ws, original.path(), ActorId::new())
            .unwrap(),
        original_actor
    );
    assert_eq!(
        store
            .actor_for_instance(&ws, copy.path(), ActorId::new())
            .unwrap(),
        copy_actor
    );
}

/// The legitimate counterpart: moving or renaming the directory must keep
/// the actor, or the device orphans the ops file it has been writing.
#[test]
fn a_moved_workspace_keeps_its_actor() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let before = parent.path().join("notes");
    std::fs::create_dir_all(&before).unwrap();
    let ws = WorkspaceId::read_or_create(&before).unwrap();
    let actor = ActorId::new();
    store.actor_for_instance(&ws, &before, actor).unwrap();

    let after = parent.path().join("notes-renamed");
    std::fs::rename(&before, &after).unwrap();
    assert_eq!(
        store
            .actor_for_instance(&ws, &after, ActorId::new())
            .unwrap(),
        actor
    );
    assert_eq!(
        store
            .actor_for_instance(&ws, &after, ActorId::new())
            .unwrap(),
        actor,
        "and the new path is the bound one from now on"
    );
}

/// A prior root we can still see holding this workspace id is *live*, so
/// the second location forks. An unreadable one gets the same treatment:
/// an extra actor costs a file, a shared actor costs data.
#[test]
fn a_still_present_prior_root_forces_a_fork() {
    let (_d, store) = store();
    let (original, ws) = workspace();
    let actor = ActorId::new();
    store
        .actor_for_instance(&ws, original.path(), actor)
        .unwrap();
    let other = TempDir::new().unwrap();
    ws.write(other.path()).unwrap();
    assert_ne!(
        store
            .actor_for_instance(&ws, other.path(), ActorId::new())
            .unwrap(),
        actor
    );
}

#[test]
fn device_actor_is_stable_and_reads_legacy_file_without_newline() {
    let (_d, store) = store();
    let first = store.device_actor().unwrap();
    assert_eq!(store.device_actor().unwrap(), first);

    // The Tauri clients used to write the ULID with no trailing newline;
    // that file must keep resolving to the same actor.
    let legacy = TempDir::new().unwrap();
    let actor = ActorId::new();
    std::fs::write(legacy.path().join("actor"), actor.to_string()).unwrap();
    let store = DeviceStore::at(legacy.path());
    assert_eq!(store.device_actor().unwrap(), actor);
    assert_eq!(
        store.device_actor().unwrap(),
        actor,
        "stamping it must not change the answer"
    );
}

#[test]
fn corrupt_actor_file_is_replaced_not_fatal() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("actor"), "not-a-ulid").unwrap();
    let store = DeviceStore::at(dir.path());
    let actor = store.device_actor().unwrap();
    assert_eq!(store.device_actor().unwrap(), actor);
}

#[test]
fn corrupt_workspace_binding_reads_as_absent() {
    let (dir, store) = store();
    let (ws_dir, ws) = workspace();
    std::fs::create_dir_all(dir.path().join("actors")).unwrap();
    std::fs::write(dir.path().join("actors").join(ws.as_str()), "garbage").unwrap();
    let actor = ActorId::new();
    assert_eq!(
        store.actor_for_instance(&ws, ws_dir.path(), actor).unwrap(),
        actor
    );
}

/// `~/.config/outl` is replicated by Migration Assistant, a Time Machine
/// restore, a VM image, chezmoi and `$HOME` on NFS. The clone must not
/// keep writing under the original's actor.
#[test]
fn a_cloned_device_store_remints_and_forks_every_actor() {
    if host_fingerprint().is_none() {
        // No OS machine identifier here (iOS / unknown target): the check
        // is inconclusive by design.
        return;
    }
    let (dir, store) = store();
    let (ws_dir, ws) = workspace();
    let original_machine = store.machine_id().unwrap();
    let original_actor = ActorId::new();
    store
        .actor_for_instance(&ws, ws_dir.path(), original_actor)
        .unwrap();

    // Same bytes, another machine: rewrite the host binding to something
    // this host can never produce.
    std::fs::write(
        dir.path().join("machine-id"),
        format!("id={original_machine}\nhost=deadbeef\n"),
    )
    .unwrap();

    let reminted = store.machine_id().unwrap();
    assert_ne!(reminted, original_machine, "the cloned id must be reminted");
    assert_eq!(store.machine_id().unwrap(), reminted, "then stable");
    assert_ne!(
        store
            .actor_for_instance(&ws, ws_dir.path(), ActorId::new())
            .unwrap(),
        original_actor,
        "bindings stamped by the machine we were cloned from are stale"
    );
}

#[test]
fn a_pre_binding_machine_id_is_stamped_in_place() {
    let (dir, store) = store();
    let legacy = ulid::Ulid::new().to_string();
    std::fs::write(dir.path().join("machine-id"), format!("{legacy}\n")).unwrap();
    assert_eq!(store.machine_id().unwrap().as_str(), legacy);
    assert_eq!(store.machine_id().unwrap().as_str(), legacy);
    if host_fingerprint().is_some() {
        let raw = std::fs::read_to_string(dir.path().join("machine-id")).unwrap();
        assert!(raw.contains("host="), "got {raw:?}");
    }
}

#[test]
fn hostile_workspace_id_cannot_escape_the_store_dir() {
    let (dir, store) = store();
    let ws_dir = TempDir::new().unwrap();
    let ws = WorkspaceId::from_raw("../../etc/passwd");
    ws.write(ws_dir.path()).unwrap();
    let actor = ActorId::new();
    assert_eq!(
        store.actor_for_instance(&ws, ws_dir.path(), actor).unwrap(),
        actor
    );
    let key = workspace_key(&ws);
    assert!(key.starts_with("h-"), "got {key}");
    assert!(dir.path().join("actors").join(&key).exists());
}

#[test]
fn plain_workspace_id_keys_verbatim() {
    let ws = WorkspaceId::from_raw("01J0000000000000000000000A");
    assert_eq!(workspace_key(&ws), "01J0000000000000000000000A");
}

/// Without `OUTL_DEVICE_DIR` the suite writes into the developer's real
/// `~/.config/outl/`, and every test binary shares the machine id that
/// arbitrates actor claims. The repo's `.cargo/config.toml` exports it for
/// every cargo-spawned process; this test is what fails when that file is
/// missing — a fresh clone, or CI.
#[test]
fn the_test_suite_runs_against_an_isolated_device_store() {
    let custom = std::env::var("OUTL_DEVICE_DIR").unwrap_or_default();
    assert!(
        !custom.is_empty(),
        "OUTL_DEVICE_DIR is unset — the repo's .cargo/config.toml is missing or was not \
         picked up, so this test run is writing into the real device store"
    );
    assert_eq!(device_dir(), PathBuf::from(&custom));
    let real = dirs::home_dir().map(|h| h.join(".config").join("outl"));
    assert_ne!(Some(device_dir()), real, "must not be the real store");
}
