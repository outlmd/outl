//! The automatic snapshot pass.
//!
//! `[backup] enabled` defaults to **on**, which was a promise nothing
//! kept: [`super::snapshot_best_effort`] had no caller outside its own
//! test, so a user who never ran `outl backup init` by hand had a
//! configuration switch saying "backups are on" and no backups at all.
//! This module is the pass that makes the default true.
//!
//! Three properties it must have, in order:
//!
//! 1. **Never on the edit path.** A snapshot walks the whole workspace
//!    and forks a process; on a large graph that is seconds. It runs on
//!    a background thread ([`spawn_auto_pass`]) so a slow disk can only
//!    ever delay a backup, never a keystroke.
//! 2. **Throttled by the git history, not by a state file.** The floor
//!    between snapshots is derived from the newest commit's timestamp
//!    (`git log -1 --format=%ct`). A separate "last run" file would be
//!    one more thing to keep honest — and one more thing to accidentally
//!    place on the sync surface.
//! 3. **Best-effort, loudly logged.** Every failure lands in a `warn!`
//!    and returns `None`.
//!
//! The pass also **creates the repository on first run**. That is safe
//! precisely because the repository is device-local and outside the
//! workspace (see the parent module): auto-initialising cannot surprise
//! a user by turning their notes folder into a git repo, and without it
//! `enabled = true` would still mean "nothing happens until you run a
//! command you have not heard of".

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::warn;

use super::{BackupEntry, BackupError, BackupRepo};

/// How long a client waits before its first automatic snapshot.
///
/// Boot is the busiest moment a client has (index build, reconcile,
/// first paint); adding a whole-workspace `git add` to it would be
/// felt. Short enough that even a brief session still gets one.
pub const STARTUP_DELAY: Duration = Duration::from_secs(60);

/// Take a snapshot when the configured interval has elapsed.
///
/// Returns `None` when backups are off, when the floor has not passed
/// yet, when nothing changed, or when anything failed — the caller
/// cannot act on the difference, and none of them is worth interrupting
/// the user over.
///
/// `interval_minutes` is `[backup] interval_minutes` (`outl-config`);
/// this crate takes the two values rather than the config struct so it
/// stays free of a dependency on `outl-config`.
pub fn maybe_snapshot(root: &Path, enabled: bool, interval_minutes: u64) -> Option<BackupEntry> {
    if !enabled {
        return None;
    }
    match run_on(&BackupRepo::for_workspace(root), interval_minutes) {
        Ok(entry) => entry,
        Err(e) => {
            warn!("automatic backup skipped: {e}");
            None
        }
    }
}

/// [`maybe_snapshot`] against an explicit repository — the form the
/// tests drive, so they never resolve the real device directory.
pub(crate) fn run_on(
    repo: &BackupRepo,
    interval_minutes: u64,
) -> Result<Option<BackupEntry>, BackupError> {
    if !repo.is_initialized() {
        repo.init()?;
    }
    if !due(repo, interval_minutes)? {
        return Ok(None);
    }
    repo.snapshot(&default_message())
}

/// Has `interval_minutes` passed since the newest snapshot?
///
/// A repository with no commits is always due. A commit dated in the
/// future (clock skew between devices sharing a folder, a machine whose
/// clock jumped) reads as "0 seconds ago" and defers — deferring a
/// backup is recoverable, and committing on every pass because a clock
/// lied is not.
fn due(repo: &BackupRepo, interval_minutes: u64) -> Result<bool, BackupError> {
    let Some(last) = repo.last_snapshot_epoch()? else {
        return Ok(true);
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let elapsed = now.saturating_sub(last).max(0) as u64;
    Ok(elapsed >= interval_minutes.saturating_mul(60))
}

fn default_message() -> String {
    format!(
        "outl snapshot {}",
        crate::clock::now_local().format("%Y-%m-%d %H:%M")
    )
}

/// The workspace root the automatic pass should snapshot *right now*.
///
/// A shared, swappable cell rather than a captured `PathBuf`, because the
/// Tauri clients open their workspace asynchronously and replace it at
/// runtime. Capturing the root would force one supervisor thread per
/// workspace ever opened, each snapshotting a root the user has left,
/// with no handle to stop any of them. One thread for the process that
/// reads the current root at each pass cannot leak that way.
///
/// `None` means no workspace has been opened yet (boot not finished, or
/// the first open failed); the pass then does nothing. A failed open
/// after a successful one leaves the previous root in place, because the
/// client still has that workspace published. Cloning shares the cell, so the
/// client keeps one clone to [`set`](Self::set) and hands the other to
/// [`spawn_auto_pass`].
#[derive(Debug, Clone, Default)]
pub struct BackupRoot(Arc<Mutex<Option<PathBuf>>>);

impl BackupRoot {
    /// A cell naming no workspace: passes do nothing until `set`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Point the supervisor at `root`, or at nothing.
    pub fn set(&self, root: Option<PathBuf>) {
        *self.lock() = root;
    }

    /// The workspace the next pass will snapshot, if one is open.
    pub fn get(&self) -> Option<PathBuf> {
        self.lock().clone()
    }

    // A poisoned lock only means another thread panicked while holding a
    // `PathBuf`; the value is still whole, and refusing to back up
    // because of it would turn one panic into silent data-loss exposure.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One supervisor pass: run `snapshot` against the open workspace, or do
/// nothing when none is open. Returns whether it ran.
///
/// Split out of the loop so "no workspace open is a no-op" is a test
/// rather than a claim, without spawning a thread or forking `git`.
fn run_pass(root: &BackupRoot, snapshot: impl FnOnce(&Path)) -> bool {
    match root.get() {
        Some(root) => {
            snapshot(&root);
            true
        }
        None => false,
    }
}

/// Run [`maybe_snapshot`] on a background thread, forever, against
/// whichever workspace `root` currently names.
///
/// The single wiring point for a client: one call at startup and the
/// workspace has periodic backups. A no-op when `enabled` is false, so a
/// caller doesn't need to branch.
///
/// Deliberately a detached daemon thread with no handle and no shutdown
/// channel. It only ever reads the workspace and writes to a repository
/// outside it, so being killed at process exit costs at most one
/// snapshot, and the floor is derived from git, so the next launch
/// takes it. What replaces a handle is [`BackupRoot`]: switching
/// workspaces changes what the one thread snapshots instead of needing a
/// second thread.
///
/// Not called on the quit path either. Blocking an exit on `git add` of
/// a large workspace is a worse bug than a snapshot arriving one session
/// late.
pub fn spawn_auto_pass(root: BackupRoot, enabled: bool, interval_minutes: u64) {
    if !enabled {
        return;
    }
    // A zero interval would spin: treat it as "as often as the pass can
    // reasonably run" rather than "continuously".
    let interval = Duration::from_secs(interval_minutes.max(1).saturating_mul(60));
    std::thread::spawn(move || {
        std::thread::sleep(STARTUP_DELAY);
        loop {
            run_pass(&root, |root| {
                maybe_snapshot(root, true, interval_minutes);
            });
            std::thread::sleep(interval);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supervisor_follows_a_workspace_switch() {
        let root = BackupRoot::new();
        root.set(Some(PathBuf::from("/a")));
        assert_eq!(root.get(), Some(PathBuf::from("/a")));
        root.set(Some(PathBuf::from("/b")));
        assert_eq!(root.get(), Some(PathBuf::from("/b")));
    }

    #[test]
    fn a_pass_with_no_workspace_open_is_a_no_op() {
        let root = BackupRoot::new();
        assert_eq!(root.get(), None);
        let mut called = false;
        assert!(!run_pass(&root, |_| called = true));
        assert!(!called, "a snapshot must not start with no workspace open");
    }

    #[test]
    fn a_pass_snapshots_the_workspace_open_at_that_moment() {
        let root = BackupRoot::new();
        let mut seen = Vec::new();
        root.set(Some(PathBuf::from("/a")));
        assert!(run_pass(&root, |r| seen.push(r.to_path_buf())));
        root.set(Some(PathBuf::from("/b")));
        assert!(run_pass(&root, |r| seen.push(r.to_path_buf())));
        root.set(None);
        assert!(!run_pass(&root, |r| seen.push(r.to_path_buf())));
        assert_eq!(seen, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn clones_share_the_cell() {
        let mine = BackupRoot::new();
        let supervisors = mine.clone();
        mine.set(Some(PathBuf::from("/a")));
        assert_eq!(supervisors.get(), Some(PathBuf::from("/a")));
    }
}
