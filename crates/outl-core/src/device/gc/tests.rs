//! What the device-store GC may and may not delete.
//!
//! Most of these pin a *refusal*. A binding wrongly dropped forks the
//! workspace's actor on its next open — the failure the device store
//! exists to prevent — so the tests that keep this honest are the ones
//! asserting nothing happened.

use std::time::Duration;

use super::*;
use tempfile::TempDir;

const TTL: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// A store with an `actors/` dir ready to be filled by hand — these tests
/// are about the *records*, not about how `actor_for_instance` writes them.
fn store() -> (TempDir, DeviceStore) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("actors")).unwrap();
    let store = DeviceStore::at(dir.path());
    (dir, store)
}

/// Write a binding naming `root`, aged `age` by backdating its mtime.
///
/// Mirrors `actor_record`: the writer stamps the root's filesystem
/// device (`dev=`) while the root is still there to ask, so a root that
/// does not exist at binding time gets the pre-stamp legacy shape.
fn binding(store: &DeviceStore, name: &str, root: &Path, age: Duration) -> PathBuf {
    let path = store.dir().join("actors").join(name);
    let mut contents = format!(
        "actor=01J0000000000000000000000A\nroot={}\nmachine=01J0000000000000000000000B\n",
        root.display()
    );
    if let Some(dev) = device_of(root) {
        contents.push_str(&format!("dev={dev}\n"));
    }
    std::fs::write(&path, contents).unwrap();
    backdate(&path, age);
    path
}

fn backdate(path: &Path, age: Duration) {
    let when = SystemTime::now() - age;
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when)).unwrap();
}

fn verdict(store: &DeviceStore, path: &Path) -> BindingVerdict {
    store
        .actor_bindings(TTL)
        .unwrap()
        .into_iter()
        .find(|b| b.path == path)
        .expect("binding listed")
        .verdict
}

#[test]
fn a_binding_whose_workspace_is_still_there_is_live() {
    let (_d, store) = store();
    let ws = TempDir::new().unwrap();
    let path = binding(&store, "live", ws.path(), Duration::from_secs(0));
    assert_eq!(verdict(&store, &path), BindingVerdict::Live);
}

/// Age alone never makes a binding prunable. A workspace opened once two
/// years ago and still on disk is an ordinary archived graph.
#[test]
fn an_ancient_binding_whose_workspace_exists_is_never_prunable() {
    let (_d, store) = store();
    let ws = TempDir::new().unwrap();
    let path = binding(
        &store,
        "ancient",
        ws.path(),
        Duration::from_secs(60 * 60 * 24 * 900),
    );
    assert_eq!(verdict(&store, &path), BindingVerdict::Live);
    assert!(store.stale_actor_bindings(TTL).unwrap().is_empty());
}

/// The case this whole module exists for: a temp workspace from a test
/// run, long gone, its parent (`/tmp`) still right there.
#[test]
fn a_deleted_workspace_past_the_ttl_is_stale() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let gone = parent.path().join("deleted-workspace");
    let path = binding(&store, "orphan", &gone, TTL + Duration::from_secs(60));

    assert_eq!(verdict(&store, &path), BindingVerdict::Stale);
    assert_eq!(store.stale_actor_bindings(TTL).unwrap().len(), 1);
    assert!(store.prune_binding(&path, TTL).unwrap());
    assert!(!path.exists());
}

/// Missing-root alone is not enough. The TTL is the window in which a
/// user who deleted a folder by mistake can restore it and keep the
/// actor they have been writing under.
#[test]
fn a_workspace_deleted_yesterday_is_kept() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let gone = parent.path().join("deleted-yesterday");
    let path = binding(&store, "recent", &gone, Duration::from_secs(60 * 60 * 24));

    assert_eq!(verdict(&store, &path), BindingVerdict::RecentlyGone);
    assert!(store.stale_actor_bindings(TTL).unwrap().is_empty());
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists(), "a refusal must not delete anything");
}

/// The limit of the TTL, pinned so nobody reads condition 3 as a
/// thirty-day grace period after a deletion. It is not one.
///
/// The mtime of a binding is when this device *bound* the workspace —
/// `actor_for_instance` only rewrites it when the root moves — so a
/// workspace bound long ago and deleted a minute ago is past the TTL the
/// moment it disappears. Every long-lived workspace is in that state,
/// which makes this the common case, not the corner.
///
/// It is acceptable because the parent check still separates a deleted
/// folder from an absent volume, and because the cost is bounded: the
/// next open forks one extra `ops-<actor>.jsonl` and loses no op.
/// Changing it means stamping every open with a `seen=` time, and this
/// test is where that decision gets made rather than drifted into.
#[test]
fn an_old_binding_whose_workspace_just_vanished_is_stale() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("long-lived");
    std::fs::create_dir_all(&root).unwrap();

    // Bound two years ago, alive until this instant.
    let path = binding(&store, "long-lived", &root, TTL * 24);
    assert_eq!(verdict(&store, &path), BindingVerdict::Live);

    std::fs::remove_dir_all(&root).unwrap();

    assert_eq!(
        verdict(&store, &path),
        BindingVerdict::Stale,
        "the TTL is the record's age, not time since the workspace went away"
    );
}

/// **Do not delete this test.** An unmounted external drive, a network
/// volume that is not up yet, and an iCloud folder not yet materialized
/// all read as "root missing" — and pruning there forks the actor of a
/// workspace that is perfectly alive, on the next plug-in.
///
/// The parent check is what tells the two apart: a deleted folder leaves
/// its parent behind, a missing mount takes the whole path with it.
#[test]
fn a_workspace_on_an_unmounted_volume_is_kept_however_old() {
    let (_d, store) = store();
    let volume = TempDir::new().unwrap();
    let root = volume.path().join("Backup").join("notes");
    // The mount point itself is absent, not just the workspace.
    let path = binding(&store, "unmounted", &root, TTL * 40);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(store.stale_actor_bindings(TTL).unwrap().is_empty());
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists());
}

/// A record we could not parse tells us nothing about its workspace, and
/// "nothing" is not permission to delete.
#[test]
fn a_record_with_no_root_is_inconclusive() {
    let (_d, store) = store();
    let path = store.dir().join("actors").join("legacy");
    std::fs::write(&path, "01J0000000000000000000000A\n").unwrap();
    backdate(&path, TTL * 10);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists());
}

/// The listing and the prune are two passes with a user in between. If
/// the workspace came back (drive plugged in, folder restored from the
/// trash) the stale verdict is stale evidence, and acting on it forks
/// that workspace's actor.
#[test]
fn a_workspace_that_reappeared_between_the_listing_and_the_prune_is_not_pruned() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("restored");
    let path = binding(&store, "restored", &root, TTL * 2);
    assert_eq!(store.stale_actor_bindings(TTL).unwrap().len(), 1);

    std::fs::create_dir_all(&root).unwrap();

    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists(), "the re-check is what makes this safe");
}

/// `identity.key` IS this device's node id and `machine-id` arbitrates
/// every binding. Neither lives under `actors/`, and the prune refuses a
/// path outside it rather than trusting its caller.
#[test]
fn nothing_outside_the_actors_directory_can_be_pruned() {
    let (dir, store) = store();
    let identity = dir.path().join("iroh").join("identity.key");
    std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
    std::fs::write(&identity, "node-id").unwrap();
    backdate(&identity, TTL * 10);

    assert!(!store.prune_binding(&identity, TTL).unwrap());
    assert!(identity.exists());

    let machine = dir.path().join("machine-id");
    std::fs::write(&machine, "id=01J0000000000000000000000B\n").unwrap();
    assert!(!store.prune_binding(&machine, TTL).unwrap());
    assert!(machine.exists());
}

/// A store that has never bound a workspace has no `actors/` dir. That is
/// a normal state, not an error worth surfacing to the user.
#[test]
fn a_store_with_no_actors_directory_lists_empty() {
    let dir = TempDir::new().unwrap();
    let store = DeviceStore::at(dir.path());
    assert!(store.actor_bindings(TTL).unwrap().is_empty());
}

/// `record.rs` composes every write in a sibling `.<name>.<pid>.<seq>`
/// scratch file. One of those belongs to another process's in-flight
/// write; listing it as a binding would report a phantom workspace and
/// offer to delete a file we do not own.
#[test]
fn in_flight_scratch_files_are_not_bindings() {
    let (_d, store) = store();
    std::fs::write(store.dir().join("actors").join(".x.123.0.tmp"), "actor=x\n").unwrap();
    assert!(store.actor_bindings(TTL).unwrap().is_empty());
}

/// The report has to name the workspace, not just the record: "1,144
/// orphans" is a number, and `root=/tmp/.tmpXYZ` is the thing that lets a
/// user recognise them as test debris.
#[test]
fn a_listed_binding_carries_the_root_it_names() {
    let (_d, store) = store();
    let ws = TempDir::new().unwrap();
    let path = binding(&store, "named", ws.path(), Duration::from_secs(0));

    let listed = store.actor_bindings(TTL).unwrap();
    let entry = listed.iter().find(|b| b.path == path).unwrap();
    assert_eq!(entry.root.as_deref(), Some(ws.path()));
}

/// **Do not delete this test.** `write_record` does not escape, and the
/// parser trims — so a workspace whose path ends in a space is written
/// faithfully and read back one character shorter. That shorter path does
/// not exist, and its parent does, which is precisely the shape this
/// module deletes on. The workspace is alive.
///
/// Before the GC existed the same leniency only cost a redundant rewrite
/// on the next open. The GC changed what it costs, so the GC is what has
/// to refuse: a record that did not survive the round trip is judged
/// `Inconclusive`, never `Stale`.
#[test]
fn a_root_that_does_not_survive_the_parser_is_never_prunable() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("trailing space ");
    std::fs::create_dir_all(&root).unwrap();

    let path = binding(&store, "spaced", &root, TTL * 4);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists(), "a live workspace must keep its binding");
}

/// The other half of the same defect: a newline in the path splits the
/// record, so `root=` comes back truncated and the tail is parsed as its
/// own line.
#[test]
fn a_root_containing_a_newline_is_never_prunable() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("two\nlines");
    std::fs::create_dir_all(&root).unwrap();

    let path = binding(&store, "newline", &root, TTL * 4);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists());
}

/// The parent-present rule assumes the root lived on the parent's own
/// filesystem. A workspace that is *itself* a mount point breaks that:
/// unmounting `/Volumes/Notes` leaves `/Volumes` behind exactly like a
/// deletion would, and pruning there forks the actor of a live external
/// volume on its next mount. The binding records the root's device id
/// while the root is there to ask, and a surviving parent on a
/// different filesystem than the recorded one is the unmount signature.
#[test]
fn a_workspace_that_was_its_own_mount_point_is_kept_after_unmount() {
    let (_d, store) = store();
    let volumes = TempDir::new().unwrap();
    // The mount point itself is absent while its parent survives.
    let root = volumes.path().join("Notes");
    // A mounted volume's device can never be its parent directory's own.
    let foreign = device_of(volumes.path()).map_or(1, |d| d.wrapping_add(1));
    let path = store.dir().join("actors").join("mount-root");
    std::fs::write(
        &path,
        format!(
            "actor=01J0000000000000000000000A\nroot={}\n\
             machine=01J0000000000000000000000B\ndev={foreign}\n",
            root.display()
        ),
    )
    .unwrap();
    backdate(&path, TTL * 4);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(store.stale_actor_bindings(TTL).unwrap().is_empty());
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists(), "an unmounted volume must keep its binding");
}

/// A `dev=` stamp we cannot read is a stamp we cannot verify, not a
/// licence to fall back to the weaker parent-only rule.
#[test]
fn an_unreadable_dev_stamp_keeps_the_binding() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("gone");
    let path = store.dir().join("actors").join("bad-dev");
    std::fs::write(
        &path,
        format!(
            "actor=01J0000000000000000000000A\nroot={}\n\
             machine=01J0000000000000000000000B\ndev=not-a-device\n",
            root.display()
        ),
    )
    .unwrap();
    backdate(&path, TTL * 4);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists());
}

/// The writer serializes the root with `Path::display()`, which replaces
/// non-Unicode path data with U+FFFD *before* the parser (and its
/// `is_lossy`) ever see the text. The stored path was never the real
/// one, so its absence proves nothing about the workspace: judged
/// `Inconclusive`, never `Stale`.
#[test]
fn a_root_the_display_serialization_already_mangled_is_never_prunable() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    // What a non-Unicode root looks like after `display()`: this
    // replacement-character rendering does not exist, its parent does.
    let root = parent.path().join("caf\u{FFFD}");
    let path = binding(&store, "mangled", &root, TTL * 4);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists());
}

/// End-to-end shape of the same defect, on a filesystem that allows
/// non-Unicode names: the workspace is alive, the recorded text is not
/// its name.
#[cfg(target_os = "linux")]
#[test]
fn a_live_non_unicode_workspace_keeps_its_binding() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join(OsStr::from_bytes(b"caf\xE9"));
    std::fs::create_dir_all(&root).unwrap();
    let path = binding(&store, "non-unicode", &root, TTL * 4);

    assert_eq!(verdict(&store, &path), BindingVerdict::Inconclusive);
    assert!(!store.prune_binding(&path, TTL).unwrap());
    assert!(path.exists(), "the workspace is alive");
}

/// The guard on `prune_binding` is advertised as what keeps
/// `iroh/identity.key` — this device's node id — out of reach. A
/// `starts_with` test passes `<dir>/actors/../iroh/identity.key`, because
/// `Path` compares components and `..` is just a component.
#[test]
fn a_traversal_path_cannot_escape_the_actors_directory() {
    let (dir, store) = store();
    let identity = dir.path().join("iroh").join("identity.key");
    std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
    std::fs::write(&identity, "node-id").unwrap();

    let escape = dir
        .path()
        .join("actors")
        .join("..")
        .join("iroh")
        .join("identity.key");

    assert!(!store.prune_binding(&escape, TTL).unwrap());
    assert!(identity.exists(), "the node id must be unreachable");
}

// ------------------------------------------------------------- scratch

const SCRATCH_TTL: Duration = Duration::from_secs(60 * 60 * 24);

fn scratch(store: &DeviceStore, name: &str, age: Duration) -> PathBuf {
    let path = store.dir().join("actors").join(name);
    std::fs::write(&path, "actor=01J0000000000000000000000A\n").unwrap();
    backdate(&path, age);
    path
}

/// `create_new_record` writes a sibling scratch file and removes it after
/// publishing. A process killed in between leaves it forever, and nothing
/// used to collect it — the same unanswered "what cleans it up?" the
/// bindings had, inside the module that exists to answer it.
#[test]
fn an_abandoned_scratch_file_is_collected_and_dropped() {
    let (_d, store) = store();
    let path = scratch(&store, ".01J0A.4242.0.new", SCRATCH_TTL * 3);

    assert_eq!(
        store.stale_scratch(SCRATCH_TTL).unwrap(),
        vec![path.clone()]
    );
    assert!(store.prune_scratch(&path, SCRATCH_TTL).unwrap());
    assert!(!path.exists());
}

/// A real write lives for microseconds, so anything inside the TTL may
/// still belong to a running process. Deleting it is survivable (the
/// writer falls through to `exclusive_create`) but pointless, so don't.
#[test]
fn a_scratch_file_from_a_live_write_is_left_alone() {
    let (_d, store) = store();
    let path = scratch(&store, ".01J0A.4242.0.new", Duration::from_secs(5));

    assert!(store.stale_scratch(SCRATCH_TTL).unwrap().is_empty());
    assert!(!store.prune_scratch(&path, SCRATCH_TTL).unwrap());
    assert!(path.exists());
}

/// The two sweeps must not see each other's subject. A scratch file names
/// no workspace, so counting one as "a binding whose workspace is gone"
/// reports a graph that never existed; and a binding is a real record,
/// never scratch.
#[test]
fn bindings_and_scratch_files_never_appear_in_each_others_listing() {
    let (_d, store) = store();
    let parent = TempDir::new().unwrap();
    let gone = parent.path().join("deleted");
    let binding_path = binding(&store, "orphan", &gone, TTL * 2);
    let scratch_path = scratch(&store, ".orphan.4242.0.tmp", SCRATCH_TTL * 3);

    let stale: Vec<_> = store
        .stale_actor_bindings(TTL)
        .unwrap()
        .into_iter()
        .map(|b| b.path)
        .collect();
    assert_eq!(stale, vec![binding_path]);
    assert_eq!(
        store.stale_scratch(SCRATCH_TTL).unwrap(),
        vec![scratch_path]
    );
}

/// `prune_scratch` is `pub` and takes a path, so it gets the same
/// traversal guard as `prune_binding`, and the same refusal to act on
/// something that is not its subject.
#[test]
fn prune_scratch_refuses_a_real_binding_and_anything_outside_actors() {
    let (dir, store) = store();
    let ws = TempDir::new().unwrap();
    let live = binding(&store, "live", ws.path(), TTL * 4);
    assert!(!store.prune_scratch(&live, SCRATCH_TTL).unwrap());
    assert!(live.exists());

    let identity = dir.path().join("iroh").join(".identity.key.tmp");
    std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
    std::fs::write(&identity, "node-id").unwrap();
    backdate(&identity, SCRATCH_TTL * 9);
    assert!(!store.prune_scratch(&identity, SCRATCH_TTL).unwrap());
    assert!(identity.exists());
}
