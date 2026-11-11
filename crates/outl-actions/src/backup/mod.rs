//! Local, versioned backups of a workspace — the safety net under
//! everything else in this crate.
//!
//! ## Why this exists
//!
//! outl's durability story is strong in the places it was designed for:
//! the op log is append-only and fsynced, every `.md` write is atomic,
//! and a delete is a move to `TRASH_ROOT` rather than a physical
//! removal. What it had no answer for is the class of failure *outside*
//! that design — a bug in a projection path, a mis-aimed `outl import`
//! over a populated workspace, a sync tool resolving a conflict the
//! wrong way, or a user deleting the wrong page and closing the app
//! (undo is in-memory and dies with the process).
//!
//! For each of those, the recovery story was "reconstruct it by hand
//! from `ops-*.jsonl`". This turns them into `outl backup restore`.
//!
//! ## Why git, and why the `git` binary
//!
//! Git gives content-addressed dedup, a real history, diffs, and
//! restore-to-a-point-in-time for free — on a directory of markdown,
//! which is exactly what git is best at. Writing a bespoke snapshot
//! format to get a worse version of that would be the wrong trade.
//!
//! We shell out to the `git` binary rather than linking `libgit2`
//! because backups must never be a reason `outl-core`'s dependents fail
//! a `cargo deny` audit (see `outl-core/CLAUDE.md` → "This crate's
//! dependency graph is public surface"), and because a workspace backed
//! up by the real `git` stays readable by every tool the user already
//! has. No git on `PATH` degrades to "backups unavailable" — never to a
//! failed edit.
//!
//! ## Where the repository lives: outside the workspace
//!
//! The git directory is **device-local**, under
//! `<outl_core::device_dir()>/backups/<slug>-<hash>.git`, and the
//! workspace is passed as `--work-tree`. Nothing is written inside the
//! workspace — no `.git/`, no `.git` pointer file. Two independent
//! reasons, both of which cost data before this layout:
//!
//! 1. **The workspace is a sync surface.** `.outl/` and everything
//!    beside it ride the file transport on every transport except iCloud
//!    (see `docs/storage.md`). A `.git/` in the workspace root means
//!    Syncthing / Dropbox / a shared NFS mount replicate the object
//!    store, `index`, `HEAD` and `index.lock` between devices under
//!    eventual consistency — a repository corrupted by exactly the
//!    mechanism the per-actor `ops-<actor>.jsonl` layout exists to route
//!    around. A device-local repo is per-device by construction, like
//!    the device store.
//! 2. **`git init` in the workspace root adopted the user's own repo.**
//!    Plenty of people already keep their notes in git. The previous
//!    version detected `<root>/.git` and used it, which meant
//!    `git add --all` clobbered their staging area, the snapshot
//!    committed onto whatever branch they had checked out, their
//!    `pre-commit` hooks ran on every automatic snapshot, and a
//!    `commit.gpgsign = true` wired to 1Password's `op-ssh-sign` turned
//!    each one into a Touch ID prompt that blocked until answered.
//!
//! A separate `--git-dir` gives us a repository we own end to end: our
//! index, our branch, our identity, no hooks (`core.hooksPath` points at
//! a directory that does not exist), no signing, and nothing for a sync
//! client to replicate.
//!
//! A `.git` left inside a workspace by an older build is **not** adopted
//! and **not** deleted — it is the user's directory. [`init`] logs where
//! the new repository lives and leaves it alone.
//!
//! ## What is backed up
//!
//! Everything that is *state*: `ops/` (the source of truth), `pages/`,
//! `journals/`, `templates/`, `assets/`, and `.outl/config.toml`.
//!
//! Not backed up: the caches that are derived and rebuildable —
//! `.outl/snapshots/`, the `.idx` sidecars, lock files, `*.tmp`. Backing
//! those up would bloat every commit with bytes that regenerate on the
//! next boot anyway.
//!
//! **A `.gitignore` in the workspace cannot silence any of that.**
//! Exclusions in the work tree beat `$GIT_DIR/info/exclude`, and a
//! negation there cannot win them back, so a `*.jsonl` line (or a
//! blanket `ops/`) dropped the op log out of every snapshot while
//! `outl backup now` kept printing commit ids and `status` kept
//! reporting a healthy history. The required paths are therefore staged
//! a second time with `git add --force`, and [`BackupRepo::snapshot`]
//! **verifies that every `ops/*.jsonl` on disk is in the resulting
//! commit** before reporting success. A history missing the source of
//! truth is a failure, not a snapshot.
//!
//! ## Automatic snapshots
//!
//! [`spawn_auto_pass`] is the one call a client makes; see the `auto`
//! module for the throttle and the "never on the edit path" rule.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use thiserror::Error;
use tracing::warn;

mod auto;
mod repo;
#[cfg(test)]
mod tests;

pub use auto::{maybe_snapshot, spawn_auto_pass, BackupRoot, STARTUP_DELAY};
pub use repo::BackupRepo;

/// Ways a backup can fail.
///
/// Callers on an edit path should treat all of these as non-fatal: a
/// backup that didn't happen is worse than one that did, and far better
/// than an edit that was refused because of it.
#[derive(Debug, Error)]
pub enum BackupError {
    /// No `git` executable on `PATH`.
    #[error("git is not available on PATH — backups are disabled")]
    GitUnavailable,

    /// The workspace has no backup repository yet.
    #[error("no backup repository for {0} — run `outl backup init` first")]
    NotInitialized(PathBuf),

    /// `git` ran but exited non-zero.
    #[error("git {command} failed: {stderr}")]
    GitFailed {
        /// The subcommand that failed (`commit`, `init`, …).
        command: String,
        /// Whatever git wrote to stderr, trimmed.
        stderr: String,
    },

    /// A restore target that resolves to the live workspace, or to a
    /// directory inside it.
    ///
    /// Extracting there is not a restore, it is an overwrite of the
    /// files being recovered — including `ops/`, the source of truth.
    #[error(
        "refusing to restore into {dest}: that is inside the live workspace ({root}).\n\
         Restoring there would overwrite the very files you are trying to recover.\n\
         Pick a directory outside the workspace, compare, then copy back what you want."
    )]
    RestoreIntoWorkspace {
        /// The resolved destination.
        dest: PathBuf,
        /// The resolved workspace root.
        root: PathBuf,
    },

    /// The snapshot ran, but the op log is not in it.
    ///
    /// The op log is the source of truth; a history without it cannot
    /// rebuild the workspace, so this is reported as a failed snapshot
    /// rather than a successful one with a footnote.
    #[error(
        "the snapshot does not contain the op log ({}) — something in the workspace is \
         excluding it from git. The backup is not trustworthy until that is fixed.",
        .missing.join(", ")
    )]
    OpLogNotCaptured {
        /// Workspace-relative paths that should be in the commit and are
        /// not.
        missing: Vec<String>,
    },

    /// Filesystem error preparing the repository.
    #[error("backup io: {0}")]
    Io(#[from] std::io::Error),
}

/// One entry in the backup history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    /// Short commit hash — what a user passes to [`restore`].
    pub id: String,
    /// ISO-8601 commit timestamp.
    pub at: String,
    /// Commit subject.
    pub message: String,
}

/// Paths git must ignore: derived caches, locks, and temp files.
///
/// Kept as a constant rather than written once at init so that a
/// repository created by an older version picks up new exclusions the
/// next time [`init`] runs (it is idempotent and rewrites this file).
///
/// Rewriting it wholesale is safe **because the repository is ours**: it
/// lives outside the workspace, so there is no user-authored exclusion
/// here to destroy. That was not true when the repo sat in the workspace
/// root and `init` truncated the user's own `.git/info/exclude`.
pub(crate) const GITIGNORE: &str = "\
# Managed by `outl backup`. Everything here is a derived cache that the
# next boot rebuilds — backing it up would bloat every commit with bytes
# that carry no information.
#
# NOTE: ops/*.jsonl is deliberately NOT ignored. The op log is the
# source of truth; the markdown is a projection of it.
.outl/snapshots/
.outl/repair-backup/
.outl/.lock
*.idx
*.tmp
.lock-*
.append.lock
.DS_Store
";

/// Directories whose contents must be in every snapshot regardless of
/// what the workspace's `.gitignore` says.
pub(crate) const REQUIRED_DIRS: &[&str] = &["ops", "pages", "journals", "templates", "assets"];

/// Individual files with the same guarantee.
pub(crate) const REQUIRED_FILES: &[&str] = &[".outl/config.toml"];

/// Pathspec exclusions applied to the forced staging pass, so
/// `add --force` can't drag the derived caches in through the back door
/// (`--force` bypasses `info/exclude` as well as `.gitignore`).
pub(crate) const FORCED_ADD_EXCLUDES: &[&str] = &[
    ":(exclude)*.idx",
    ":(exclude)*.tmp",
    ":(exclude).lock-*",
    ":(exclude).append.lock",
];

/// Author recorded on snapshots.
///
/// Deliberately **not** the user's git identity: this repository is
/// outl's, the commits are machine-made, and borrowing their
/// `user.email` would make a snapshot look like something they wrote.
pub(crate) const AUTHOR_NAME: &str = "outl backup";
/// Email paired with [`AUTHOR_NAME`].
pub(crate) const AUTHOR_EMAIL: &str = "backup@outl.app";

/// Is a usable `git` on `PATH`?
pub fn git_available() -> bool {
    run_git(&std::env::temp_dir(), &["--version"], "--version", None).is_ok()
}

/// `<device_dir>/backups/<slug>-<hash>.git` — device-local, never on the
/// workspace's sync surface.
pub(crate) fn default_git_dir(work_tree: &Path) -> PathBuf {
    let name = work_tree
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");
    let slug = outl_md::slugify(name);
    // The full path is what makes two same-named workspaces distinct.
    let digest = outl_md::hash_bytes(work_tree.to_string_lossy().as_bytes());
    let short = &digest[..digest.len().min(12)];
    outl_core::device_dir()
        .join("backups")
        .join(format!("{slug}-{short}.git"))
}

/// Where this device keeps `root`'s backup repository.
///
/// Exposed so `outl backup status` (and a curious user with plain `git`)
/// can name the directory.
pub fn repo_dir(root: &Path) -> PathBuf {
    BackupRepo::for_workspace(root).git_dir().to_path_buf()
}

/// Does `root` already have a backup repository?
pub fn is_initialized(root: &Path) -> bool {
    BackupRepo::for_workspace(root).is_initialized()
}

/// Create (or refresh) the backup repository for `root`.
pub fn init(root: &Path) -> Result<(), BackupError> {
    BackupRepo::for_workspace(root).init()
}

/// Commit the current state of the workspace.
/// See [`BackupRepo::snapshot`].
pub fn snapshot(root: &Path, message: &str) -> Result<Option<BackupEntry>, BackupError> {
    BackupRepo::for_workspace(root).snapshot(message)
}

/// Most recent backups, newest first.
pub fn list(root: &Path, limit: usize) -> Result<Vec<BackupEntry>, BackupError> {
    BackupRepo::for_workspace(root).list(limit)
}

/// Copy the workspace as it stood at `id` into `dest`.
/// See [`BackupRepo::restore`].
pub fn restore(root: &Path, id: &str, dest: &Path) -> Result<(), BackupError> {
    BackupRepo::for_workspace(root).restore(id, dest)
}

/// Take a snapshot, swallowing every failure into a `warn!`.
///
/// The form a manual caller uses when it must not propagate; the
/// automatic pass is [`maybe_snapshot`], which adds the interval floor.
/// A backup is insurance, and insurance that can block a save is a
/// liability, so nothing here is allowed to reach the user's edit path.
pub fn snapshot_best_effort(root: &Path, message: &str) -> Option<BackupEntry> {
    match snapshot(root, message) {
        Ok(entry) => entry,
        Err(e) => {
            warn!("backup skipped: {e}");
            None
        }
    }
}

/// Run `git` in `cwd`, returning stdout on success.
///
/// Scrubs the inherited `GIT_*` environment. `.current_dir()` does
/// **not** win against `GIT_DIR` / `GIT_WORK_TREE`: outl launched from a
/// git hook, a `git rebase --exec`, or any shell where those are
/// exported would otherwise run `add --all` and `commit` against
/// whatever repository the environment names, not the workspace.
pub(crate) fn run_git(
    cwd: &Path,
    args: &[&str],
    label: &str,
    index: Option<&Path>,
) -> Result<String, BackupError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args).stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    // Nothing here talks to a remote, but a credential helper reached
    // through a misconfigured repo would block forever on a terminal
    // that isn't there.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    if let Some(path) = index {
        cmd.env("GIT_INDEX_FILE", path);
    }
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BackupError::GitUnavailable
        } else {
            BackupError::Io(e)
        }
    })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(BackupError::GitFailed {
            command: label.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}
