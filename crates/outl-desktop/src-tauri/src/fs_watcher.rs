//! Cross-platform filesystem watcher for the active workspace.
//!
//! Wraps `notify` + `notify-debouncer-full` so the frontend gets a
//! single `peer-ops-changed` event when a peer writes its
//! `ops-<actor>.jsonl` (via iCloud Drive, Dropbox, Syncthing or any
//! shared filesystem).
//!
//! Replaces the iOS-only `main.mm` (`NSMetadataQuery` +
//! `NSFileCoordinator`) the mobile crate uses — `notify` is
//! cross-platform, and desktop iCloud Drive materialises files
//! automatically (no peer-file-not-yet-downloaded dance).
//!
//! ## Scope: only `ops/`
//!
//! We watch **only the `ops/` subdirectory**, not the workspace
//! root. The reason is a feedback loop the first cut hit:
//!
//! 1. `reload_workspace` runs (`peer-ops-changed` from the frontend).
//! 2. It calls `reproject_page` which writes `journals/today.md`.
//! 3. The watcher (when scoped at the root) sees the `.md` change.
//! 4. Watcher emits `peer-ops-changed`.
//! 5. Frontend reloads → goto 1.
//!
//! The user sees the window stop responding because the loop
//! saturates the Tauri command queue. Scoping the watch to `ops/`
//! and refusing to recurse means our own `.md` projections never
//! produce events, while peer-shipped jsonl files still do.
//!
//! Peer `.md` files that arrive without a corresponding ops update
//! (the user dropped a Roam export into `pages/`, or vim touched a
//! file) are picked up by the orphan scanner the way the mobile
//! crate already does — see `workspace_open::reconcile_orphan_md`.
//!
//! ## Filtering
//!
//! Inside `ops/` we still ignore our own `ops-<own_actor>.jsonl`
//! (a `Workspace::apply` write that we already know about) and
//! lock / temp files that flap during write windows.
//!
//! ## A second feedback loop: read-only access events (Linux, #474)
//!
//! `notify`'s inotify backend also reports pure reads: opening a file
//! surfaces `Access(Open(..))`, and closing one without having
//! written to it surfaces `Access(Close(Read))`. `reload_workspace`
//! opens a peer's `ops-*.jsonl` read-only to replay it, so the first
//! cut's path-only filter saw that open/close as a peer write, re-ran
//! `peer-ops-changed`, and the frontend reloaded and re-read the file
//! forever: the same symptom as the `.md` loop above, triggered by
//! reading instead of writing. `event_kind_is_interesting` drops every
//! `Access` kind except `Close(Write)` (`CLOSE_WRITE`, which only
//! fires when the file was actually modified before closing), so a
//! real completed write still emits while a replay's read never does.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use outl_core::id::ActorId;
use tauri::{AppHandle, Emitter};
use tracing::{debug, warn};

/// Concrete debouncer type used by the desktop client.
///
/// Boxing it behind a type alias keeps the `AppState` signature
/// readable and isolates the change if `notify_debouncer_full`'s
/// generic parameters ever shift.
pub(crate) type WatcherHandle = Debouncer<RecommendedWatcher, RecommendedCache>;

/// Start watching the workspace's `ops/` directory. Returns the
/// live debouncer — the caller keeps it alive (drop = stop
/// watching). When the user switches workspaces, drop the old
/// handle and call this again with the new root.
///
/// Errors propagate up so the caller can log; they're never fatal
/// (the workspace still works without a live watcher, the user just
/// has to refresh manually).
pub(crate) fn start_watcher(
    storage_root: &Path,
    own_actor: ActorId,
    app: AppHandle,
) -> notify::Result<WatcherHandle> {
    let own_ops_file = format!("ops-{own_actor}.jsonl");
    let ops_dir = storage_root.join("ops");

    let mut debouncer = new_debouncer(
        Duration::from_millis(150),
        None,
        move |result: DebounceEventResult| match result {
            Ok(events) => {
                if events_should_emit(&events, &own_ops_file) {
                    if let Err(e) = app.emit("peer-ops-changed", ()) {
                        warn!("emit peer-ops-changed: {e}");
                    }
                }
            }
            Err(errors) => {
                for e in errors {
                    warn!("fs watcher error: {e}");
                }
            }
        },
    )?;

    // NonRecursive: `ops/` is flat (one jsonl per actor). Watching
    // recursively would only invite OS-specific metadata files
    // (`.icloud` placeholders, `.DS_Store`, …) and a wider blast
    // radius without buying anything.
    debouncer.watch(&ops_dir, RecursiveMode::NonRecursive)?;
    Ok(debouncer)
}

/// Decide whether a debounced batch of events should emit
/// `peer-ops-changed`. Returns `true` as soon as one event in the
/// batch carries a peer's `ops-*.jsonl` write; otherwise `false`.
fn events_should_emit(
    events: &[notify_debouncer_full::DebouncedEvent],
    own_ops_file: &str,
) -> bool {
    events.iter().any(|ev| {
        event_kind_is_interesting(&ev.kind)
            && ev
                .paths
                .iter()
                .any(|p| path_is_interesting(p, own_ops_file))
    })
}

/// `true` unless `kind` is a pure read. `notify`'s inotify backend
/// reports opening and closing a file even when nobody wrote to it
/// (see the module doc's "read-only access events" section), and
/// `reload_workspace` does exactly that when it replays a peer's
/// ops file. `Access(Close(Write))` is the one access kind that
/// signals an actual write happened before the close, so it stays
/// interesting; every other `Access` kind is a read artefact.
fn event_kind_is_interesting(kind: &notify::EventKind) -> bool {
    use notify::event::{AccessKind, AccessMode};
    match kind {
        notify::EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        notify::EventKind::Access(_) => false,
        _ => true,
    }
}

/// `true` when `path` is a peer's `ops-*.jsonl` write (and not a
/// lock / temp / our own file). The watcher is already scoped to
/// `ops/` by `start_watcher`, so `path` is always something
/// directly inside that directory.
fn path_is_interesting(path: &Path, own_ops_file: &str) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with(".outl-") || name.ends_with(".lock") || name.ends_with(".tmp") {
        return false;
    }
    if name.starts_with("ops-") && name.ends_with(".jsonl") {
        return name != own_ops_file;
    }
    false
}

/// Replace the debouncer in `slot` with `next`, dropping the old
/// one (which stops watching). Used when the user switches
/// workspaces at runtime.
pub(crate) fn swap_watcher(
    slot: &parking_lot::Mutex<Option<WatcherHandle>>,
    next: Option<WatcherHandle>,
) {
    let mut guard = slot.lock();
    *guard = next;
    debug!("fs watcher slot swapped");
}

/// Convenience: drop the active watcher, ignoring whether one was
/// running. Lets call sites avoid `Some(None)` ladders.
#[allow(dead_code)]
pub(crate) fn stop_watcher(slot: &parking_lot::Mutex<Option<WatcherHandle>>) {
    swap_watcher(slot, None);
}

/// Shorthand path helper used by the integration layer when it
/// needs to surface the watched root in logs.
#[allow(dead_code)]
pub(crate) fn watched_root_label(p: &Path) -> String {
    let mut buf = PathBuf::from(p);
    if !buf.is_absolute() {
        if let Ok(cwd) = std::env::current_dir() {
            buf = cwd.join(buf);
        }
    }
    buf.display().to_string()
}

#[cfg(test)]
mod tests {
    //! Filter-policy tests. The actual `notify` watcher is exercised
    //! end-to-end by the manual smoke (`cargo tauri dev` + a second
    //! actor writing into `ops/`); the unit tests here pin down the
    //! pure logic that decides "this event matters" so a regression
    //! caught by `cargo test` lands long before a user notices a
    //! window stop responding from a feedback loop.
    use super::*;
    use notify::event::{
        AccessKind, AccessMode, CreateKind, DataChange, EventKind, ModifyKind, RemoveKind,
    };
    use notify::Event;
    use notify_debouncer_full::DebouncedEvent;
    use std::time::Instant;

    const OWN: &str = "ops-01HSELFAAAAAAAAAAAAAAAAAAA.jsonl";

    fn debounced<P: Into<PathBuf>>(kind: EventKind, path: P) -> DebouncedEvent {
        DebouncedEvent::new(Event::new(kind).add_path(path.into()), Instant::now())
    }

    // ── path_is_interesting ─────────────────────────────────────────

    #[test]
    fn peer_ops_file_is_interesting() {
        let p = PathBuf::from("/ws/ops/ops-01HPEER000000000000000000.jsonl");
        assert!(path_is_interesting(&p, OWN));
    }

    #[test]
    fn own_ops_file_is_ignored() {
        // Loud failure mode of the original bug: our own write
        // re-triggered the watcher. Keep the guard exact-match.
        let p = PathBuf::from(format!("/ws/ops/{OWN}"));
        assert!(!path_is_interesting(&p, OWN));
    }

    #[test]
    fn workspace_lock_is_ignored() {
        let p = PathBuf::from("/ws/ops/.outl-lock");
        assert!(!path_is_interesting(&p, OWN));
    }

    #[test]
    fn temp_and_lock_suffixes_are_ignored() {
        // Atomic writes land as `.tmp` then rename; `.lock` flaps
        // exist briefly during cross-process write windows. Either
        // would cause a phantom reload if not filtered.
        assert!(!path_is_interesting(
            &PathBuf::from("/ws/ops/ops-x.jsonl.tmp"),
            OWN
        ));
        assert!(!path_is_interesting(
            &PathBuf::from("/ws/ops/ops-x.jsonl.lock"),
            OWN
        ));
    }

    #[test]
    fn arbitrary_files_in_ops_are_ignored() {
        // The watcher is scoped to `ops/` and `ops/` is supposed to
        // be ops-*.jsonl files only. A stray `notes.txt` someone
        // drops in there is not a workspace event.
        assert!(!path_is_interesting(
            &PathBuf::from("/ws/ops/notes.txt"),
            OWN
        ));
        assert!(!path_is_interesting(
            &PathBuf::from("/ws/ops/.DS_Store"),
            OWN
        ));
    }

    #[test]
    fn ops_prefix_without_jsonl_suffix_is_ignored() {
        // Defensive: `ops-something.bak` happens when a power user
        // makes a manual backup. Not an actor file → don't emit.
        let p = PathBuf::from("/ws/ops/ops-backup.bak");
        assert!(!path_is_interesting(&p, OWN));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_filename_is_ignored() {
        // `to_str()` returns `None` for non-UTF-8 names — bail
        // safely instead of crashing the watcher callback.
        //
        // The test is unix-gated because constructing a non-UTF-8
        // path requires `OsStringExt::from_vec` (only available on
        // Unix). On Windows `OsString` is WTF-16; UTF-8-invalid
        // paths exist but need a different construction path and the
        // policy under test (`name.to_str()? -> None -> false`)
        // covers both targets identically.
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![0xff, 0xfe, b'a']);
        let mut p = PathBuf::from("/ws/ops");
        p.push(raw);
        assert!(!path_is_interesting(&p, OWN));
    }

    // ── events_should_emit ──────────────────────────────────────────

    #[test]
    fn batch_with_a_peer_modify_emits() {
        let events = vec![debounced(
            EventKind::Modify(ModifyKind::Any),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(events_should_emit(&events, OWN));
    }

    #[test]
    fn batch_with_a_peer_create_emits() {
        let events = vec![debounced(
            EventKind::Create(CreateKind::File),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(events_should_emit(&events, OWN));
    }

    #[test]
    fn batch_with_a_peer_remove_emits() {
        // Removes also count — the peer or sync layer wiped their
        // jsonl and the workspace view should re-derive.
        let events = vec![debounced(
            EventKind::Remove(RemoveKind::File),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(events_should_emit(&events, OWN));
    }

    #[test]
    fn batch_of_only_own_writes_does_not_emit() {
        // The exact failure mode the scope-to-ops/ change fixed:
        // self-triggered events must not reach the frontend.
        let events = vec![
            debounced(EventKind::Modify(ModifyKind::Any), format!("/ws/ops/{OWN}")),
            debounced(EventKind::Modify(ModifyKind::Any), format!("/ws/ops/{OWN}")),
        ];
        assert!(!events_should_emit(&events, OWN));
    }

    #[test]
    fn one_peer_event_in_a_noisy_batch_still_emits() {
        // Debouncer hands batches; a single peer write among many
        // own writes must still reach the frontend.
        let events = vec![
            debounced(EventKind::Modify(ModifyKind::Any), format!("/ws/ops/{OWN}")),
            debounced(
                EventKind::Modify(ModifyKind::Any),
                "/ws/ops/ops-01HPEER000000000000000000.jsonl",
            ),
            debounced(EventKind::Modify(ModifyKind::Any), format!("/ws/ops/{OWN}")),
        ];
        assert!(events_should_emit(&events, OWN));
    }

    #[test]
    fn empty_batch_does_not_emit() {
        assert!(!events_should_emit(&[], OWN));
    }

    // ── Linux inotify access events (issue #474) ────────────────────

    #[test]
    fn peer_access_open_does_not_emit() {
        // `reload_workspace` opens a peer's ops file read-only to
        // replay it. On Linux that open() itself is reported by
        // inotify as Access(Open), which must not read as a peer
        // write, or every reload re-triggers itself forever.
        let events = vec![debounced(
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(!events_should_emit(&events, OWN));
    }

    #[test]
    fn peer_access_close_read_does_not_emit() {
        // CLOSE_NOWRITE: the file was closed without being modified,
        // i.e. a pure read. Same loop as the open() case above.
        let events = vec![debounced(
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(!events_should_emit(&events, OWN));
    }

    #[test]
    fn peer_access_close_write_still_emits() {
        // CLOSE_WRITE: the file was modified before being closed, a
        // real completed write from the peer or sync layer, not our
        // own reload reading it back.
        let events = vec![debounced(
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(events_should_emit(&events, OWN));
    }

    #[test]
    fn batch_with_a_peer_modify_data_emits() {
        let events = vec![debounced(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            "/ws/ops/ops-01HPEER000000000000000000.jsonl",
        )];
        assert!(events_should_emit(&events, OWN));
    }

    #[test]
    fn own_ops_file_access_open_does_not_emit() {
        // Belt and suspenders: our own file is already filtered by
        // path, but it must stay filtered once access events are
        // read-interesting for peer files too.
        let events = vec![debounced(
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            format!("/ws/ops/{OWN}"),
        )];
        assert!(!events_should_emit(&events, OWN));
    }

    // ── watched_root_label ──────────────────────────────────────────

    #[test]
    fn watched_root_label_preserves_absolute_paths() {
        // The literal absolute path differs per platform (Windows
        // needs a drive letter), so pick the cwd as a known-absolute
        // anchor that always round-trips. Real-world callers feed
        // `watched_root_label` an absolute workspace path; the
        // function is just supposed to leave it alone.
        let absolute = std::env::current_dir().unwrap();
        let labelled = watched_root_label(&absolute);
        assert_eq!(PathBuf::from(labelled), absolute);
    }

    #[test]
    fn watched_root_label_promotes_relative_to_absolute() {
        // Relative paths get resolved against cwd. We don't assert
        // the cwd content, just that the result is absolute.
        let relative = PathBuf::from("rel/dir");
        let labelled = watched_root_label(&relative);
        assert!(
            PathBuf::from(&labelled).is_absolute(),
            "expected absolute label, got {labelled:?}"
        );
    }
}
