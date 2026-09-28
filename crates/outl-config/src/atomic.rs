//! Getting bytes onto disk so a reader never sees half of them.
//!
//! Split out of `lib.rs`, which owns the other half of a save: reading
//! `config.toml`, classifying it, and refusing to write over one it
//! could not read (issue #284). This module answers a different
//! question — *given that the write is allowed, how does it land?* —
//! and the answer is: compose in a scratch file nobody else is holding,
//! `fsync` it, rename it over the real name, `fsync` the directory.
//!
//! The scratch name is **per write**. `config.toml` has several writers
//! by design (the TUI and the desktop app share it), and a shared
//! scratch name is a shared inode — see [`tmp_path`] for what that cost
//! and why a zero-byte config is the worst place for this file to land.

use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::SaveError;

/// Write `body` at `path`, atomically.
///
/// Composes into a scratch sibling, `fsync`s it, renames it over `path`,
/// then `fsync`s the parent directory so the rename itself is durable.
/// Both `fsync`s matter for this file — a config that comes back
/// zero-length after a power loss silently resets the user to defaults
/// (theme, vim mode, last workspace), and the code before this had
/// neither.
///
/// The caller creates the parent directory and decides whether the write
/// is allowed at all; this only publishes.
pub(crate) fn publish(path: &Path, body: &str) -> Result<(), SaveError> {
    let guard = TempFile::new(tmp_path(path));

    {
        let mut file =
            fs::File::create(guard.path()).map_err(|e| SaveError::io(guard.path(), e))?;
        file.write_all(body.as_bytes())
            .map_err(|e| SaveError::io(guard.path(), e))?;
        file.sync_all()
            .map_err(|e| SaveError::io(guard.path(), e))?;
    }
    fs::rename(guard.path(), path).map_err(|e| SaveError::io(path, e))?;
    guard.keep();
    // After the publish, so its cost is off the failure path and it can
    // never touch a scratch this call still depends on.
    sweep_stale_scratch(path);

    // Best-effort: Windows refuses to open a directory as a file, and
    // failing the whole save there would be worse than the durability
    // gap being closed.
    if let Some(dir) = path.parent() {
        if let Ok(handle) = fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    Ok(())
}

/// Scratch path for **one** in-flight rewrite of `path`: the filename
/// with a leading dot, a `.tmp` marker, and a ULID unique to this call,
/// so `config.toml` becomes `.config.toml.tmp.01K…`.
///
/// **The ULID is the whole point.** This file has several writers by
/// design — the TUI and the desktop app read and write the same
/// `~/.config/outl/config.toml` (see the crate doc) — and one shared
/// scratch name means one shared *inode*: writer B's `File::create`
/// truncates the body A already `fsync`ed, A's `rename` publishes those
/// zero bytes, and B goes on writing through a descriptor that now points
/// at the published `config.toml` while its own rename fails `ENOENT`.
/// The user sees "could not write", and what is on disk is a zero-byte
/// config.
///
/// **That landing spot is the worst one this crate has**, which is why
/// the fix is here rather than in a fourth `crate::ConfigSource` verdict.
/// Every field carries `#[serde(default)]`, so `""` deserializes into a
/// whole `Config`: a zero-byte file is `crate::ConfigSource::Parsed`, the
/// issue #284 write guard finds nothing to refuse, and the next save
/// writes defaults over the user's theme, `vim_mode` and
/// `[sync] transport`. That is precisely the loss #284 added a guard for,
/// reached through a door the guard does not watch.
///
/// **And calling zero bytes `Unreadable` would not be that fix.** An
/// empty `config.toml` is a legitimate config meaning "all defaults" —
/// `touch ~/.config/outl/config.toml` is how a user starts one by hand,
/// and the behaviour table in `CLAUDE.md` has said so since the file
/// existed. Refusing to save over it would lock that user out of every
/// settings toggle with a message telling them to repair a file that is
/// not broken: a guard turned into a wall. So the length stays
/// uninterpreted and the cause is removed instead — a scratch name nobody
/// else can be holding.
///
/// Same fix and same reason as `outl_core::snapshot::scratch_path` and
/// `outl_core::storage::sidecar`'s `tmp_path_for`, both of which took the
/// identical `rename …: No such file or directory` in production. The
/// leading dot is kept so a scratch a killed process abandons stays out
/// of a plain `ls` of the config directory.
fn tmp_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default();
    let mut scratch = std::ffi::OsString::from(if name.as_encoded_bytes().starts_with(b".") {
        ""
    } else {
        "."
    });
    scratch.push(name);
    scratch.push(format!("{SCRATCH_MARKER}{}", ulid::Ulid::new()));
    path.with_file_name(scratch)
}

/// What separates a scratch name from the published one it belongs to.
/// Written once so [`tmp_path`] and [`sweep_stale_scratch`] cannot
/// disagree about which files are this crate's to delete.
const SCRATCH_MARKER: &str = ".tmp.";

/// How long an abandoned scratch file is left alone before it is swept.
///
/// It is a margin, not a deadline. A save composes and publishes in
/// milliseconds, so anything this old was abandoned by a process that
/// died between the `create` and the `rename` — and the margin is what
/// guarantees a *live* writer's scratch is never unlinked under it, which
/// would fail that writer's rename with the very `ENOENT` this module
/// exists to stop producing.
///
/// 24h to match `outl_core::snapshot::gc::STALE_TMP_TTL` and
/// `outl_core::device::gc::STALE_SCRATCH_TTL`; a third number for the
/// same question would be a third thing to keep straight.
const STALE_SCRATCH_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60 * 24);

/// Unlink scratch siblings of `path` older than [`STALE_SCRATCH_TTL`].
///
/// A unique scratch name removes the shared-inode race and hands back a
/// new question: nothing recycles the name any more, so a process killed
/// mid-save leaves its scratch in `~/.config/outl/` forever (root
/// `CLAUDE.md` invariant 9 — state that moves still needs an answer for
/// what cleans it up). [`TempFile`] covers every in-process exit path;
/// this covers the one it cannot.
///
/// **Best-effort, and deliberately narrow.** Every error is swallowed:
/// failing a save the user asked for because a directory listing failed
/// would be a worse trade than an orphan. A name has to start with the
/// exact dotted prefix *and* carry [`SCRATCH_MARKER`] to be a candidate,
/// so `config.toml` itself can never match — it has neither.
fn sweep_stale_scratch(path: &Path) {
    let (Some(dir), Some(prefix)) = (path.parent(), scratch_prefix(path)) else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|m| m.elapsed().map_err(io::Error::other))
            .is_ok_and(|age| age > STALE_SCRATCH_TTL);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The fixed head of every scratch name [`tmp_path`] derives from `path`
/// — everything before the per-call ULID.
fn scratch_prefix(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let dot = if name.starts_with('.') { "" } else { "." };
    Some(format!("{dot}{name}{SCRATCH_MARKER}"))
}

/// A temp file that deletes itself unless [`TempFile::keep`] is called.
///
/// **This is a deliberate third copy, and it is the only one that had no
/// alternative.** The canonical guard is `outl_md::atomic::TempFile`
/// (which `outl-actions` and `outl-sync-iroh` both use); a second,
/// module-private one lives in `outl_core::storage::sidecar`. This crate
/// is a leaf — it depends on no other `outl-*` crate, and inverting that
/// so a config parser pulls in the markdown pipeline (comrak) and the
/// CRDT kernel (yrs) to reuse twenty lines is a worse trade than the
/// duplication.
///
/// Keep the three in sync by hand, and prefer the `outl-md` one for any
/// new call site that can reach it. If this crate ever gains an
/// `outl-md` edge for another reason, delete this copy.
struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// The rename succeeded; there is nothing left at this path.
    fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Every scratch sibling in `dir`, whatever ULID it ended up with.
    ///
    /// Matched on the `.tmp.` marker rather than the extension: the
    /// extension is the per-write ULID now, and a filter that only
    /// accepted `*.tmp` would quietly stop seeing leaked scratch files —
    /// a cleanup test that can no longer observe the thing it asserts
    /// about.
    pub(crate) fn leftover_temps(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<_> = fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(SCRATCH_MARKER))
            })
            .collect();
        found.sort();
        found
    }

    /// Fail a save *after* the guard has approved it, with a real I/O
    /// error and no injection: a read-only parent directory lets
    /// `create_dir_all` through (the directory exists) and stops
    /// `File::create` from making the scratch file.
    ///
    /// This replaced planting a directory on the scratch path, which
    /// worked only while the scratch name was predictable. A test that
    /// needs to know the next ULID is a test pinning the bug shut.
    pub(crate) fn with_readonly_dir<T>(dir: &Path, body: impl FnOnce() -> T) -> T {
        use std::os::unix::fs::PermissionsExt as _;
        let original = fs::metadata(dir).expect("stat dir").permissions();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).expect("lock dir");
        let out = body();
        fs::set_permissions(dir, original).expect("unlock dir");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::{load_from, save_to, Config};
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn a_successful_save_leaves_no_scratch_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        save_to(&path, &Config::default()).unwrap();
        assert_eq!(leftover_temps(tmp.path()), Vec::<PathBuf>::new());
    }

    /// The failure path: a real I/O error, no injection — a read-only
    /// parent directory makes `File::create` fail. The old code cleaned up
    /// on no failure path at all, leaving a scratch file in
    /// `~/.config/outl/` after any transient error.
    ///
    /// It used to fail at the rename by planting a directory on `path`
    /// itself. That no longer reaches the rename: a directory where
    /// `config.toml` should be is an unreadable config, so the guard
    /// refuses before any scratch file exists (issue #284) — and a test
    /// that passes because nothing was attempted is not a test of the
    /// cleanup.
    #[test]
    fn a_failed_save_leaves_no_scratch_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        save_to(&path, &Config::default()).expect("seed a readable config");

        with_readonly_dir(tmp.path(), || {
            save_to(&path, &Config::default()).expect_err("a locked directory must fail the save");
        });
        assert_eq!(
            leftover_temps(tmp.path()),
            Vec::<PathBuf>::new(),
            "no scratch file may survive a failed save"
        );
    }

    /// A failed save must not damage the config already on disk — the
    /// whole reason this publishes by rename.
    #[test]
    fn a_failed_save_leaves_the_previous_config_intact() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.theme.preset = "dracula".into();
        save_to(&path, &cfg).unwrap();

        with_readonly_dir(tmp.path(), || {
            save_to(&path, &Config::default()).expect_err("a locked directory must fail the save");
        });

        assert_eq!(load_from(&path).theme.preset, "dracula");
    }

    #[test]
    fn the_scratch_file_is_a_dotfile() {
        let name = tmp_path(Path::new("/x/config.toml"));
        let name = name.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".config.toml.tmp."), "{name}");
    }

    /// The concurrency fix, as a unit: two calls must never hand back the
    /// same path. A shared name is a shared inode, and the cost of that
    /// is a published zero-byte config — see [`tmp_path`].
    ///
    /// The end-to-end proof is `tests/concurrent_save.rs`; this is the
    /// part that cannot be flaky.
    #[test]
    fn every_scratch_name_is_its_own() {
        let path = Path::new("/x/config.toml");
        let names: std::collections::BTreeSet<_> = (0..64).map(|_| tmp_path(path)).collect();
        assert_eq!(names.len(), 64, "a scratch name is per write, not per file");
    }

    /// A leading dot is not doubled for a path that already has one.
    #[test]
    fn a_dotted_config_name_keeps_one_dot() {
        let name = tmp_path(Path::new("/x/.config.toml"));
        let name = name.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".config.toml.tmp."), "{name}");
    }

    /// The sweep answers "what cleans up an abandoned scratch" without
    /// becoming a way to lose a live one — root `CLAUDE.md` invariant 9.
    ///
    /// Three verdicts in one test on purpose: the three are one rule, and
    /// a sweep that only proves it deletes something is the dangerous
    /// half.
    #[test]
    fn the_sweep_takes_only_scratch_files_nobody_is_writing() {
        use std::time::{Duration, SystemTime};

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        save_to(&path, &Config::default()).expect("seed a readable config");

        let abandoned = tmp_path(&path);
        let in_flight = tmp_path(&path);
        fs::write(&abandoned, b"half a config").unwrap();
        fs::write(&in_flight, b"another writer, right now").unwrap();
        let long_ago = SystemTime::now() - (STALE_SCRATCH_TTL + Duration::from_secs(60));
        fs::File::options()
            .write(true)
            .open(&abandoned)
            .and_then(|f| f.set_times(fs::FileTimes::new().set_modified(long_ago)))
            .expect("age the abandoned scratch");

        sweep_stale_scratch(&path);

        assert!(!abandoned.exists(), "a scratch past the TTL must be swept");
        assert!(
            in_flight.exists(),
            "a fresh scratch belongs to a live writer; unlinking it fails \
             that writer's rename with the ENOENT this module exists to \
             stop producing"
        );
        assert!(path.exists(), "the published config is never a candidate");
    }

    #[test]
    fn the_guard_unlinks_an_unkept_temp() {
        let tmp = TempDir::new().unwrap();
        let scratch = tmp.path().join(".scratch.tmp");
        fs::write(&scratch, b"half").unwrap();
        drop(TempFile::new(scratch.clone()));
        assert!(!scratch.exists(), "dropping an armed guard must unlink");
    }

    #[test]
    fn the_guard_leaves_a_kept_temp_alone() {
        let tmp = TempDir::new().unwrap();
        let scratch = tmp.path().join(".scratch.tmp");
        fs::write(&scratch, b"published").unwrap();
        TempFile::new(scratch.clone()).keep();
        assert!(scratch.exists(), "keep() must disarm the unlink");
    }
}
