//! Atomic file writes — `write-temp + rename` pattern.
//!
//! `fs::write` is two operations under the hood: truncate + write. A
//! crash between them leaves a partial file. For outl this could mean
//! a `.md` with half a page or a sidecar with a broken JSON. Either
//! way you'd need [`crate::reconcile`] to clean up.
//!
//! The fix is universal in POSIX: write to a sibling `*.tmp` then
//! `rename(tmp, final)`. `rename` is atomic — readers see either the
//! old file or the new file, never a half-written one.
//!
//! On Windows the same `std::fs::rename` works on the same volume
//! (which `.outl/` always is).
//!
//! The read counterpart is [`read_for_rewrite`] — see its docs for why
//! `read_to_string(..).unwrap_or_default()` is never acceptable on a
//! path you are about to write back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A temp file that deletes itself unless [`TempFile::keep`] is called.
///
/// The publish-by-rename pattern creates a scratch file and then has
/// several fallible steps before the rename makes it real. Cleaning up
/// on *some* of those paths is the bug this type exists to remove: an
/// audit found five call sites across four crates doing exactly that,
/// and the worst of them leaked a non-dotted `pages/<name>.md.tmp` that
/// a `transport = "file"` workspace then replicated to every device as
/// permanent junk no repair pass sweeps.
///
/// `Drop` covers every in-process exit: an early `?`, a panic, and — for
/// an `async` writer — a cancelled future dropped between the write and
/// the rename. What it cannot cover is a process that never runs code
/// again (`SIGKILL`, iOS jetsam, power loss), which is why `tmp_path`
/// also hides the scratch behind a leading dot: a temp abandoned that
/// way stays off the file-sync surface.
///
/// **Sibling implementation:** `outl_core::storage::sidecar::TempFile`
/// is the same type, private to that module. Two copies exist because
/// `outl-config` and `outl-core` share no dependency edge with each
/// other or with this crate in the right direction; see this module's
/// entry in `docs/contributing.md` before adding a third.
pub struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    /// Arm a guard over `path`. Nothing is created here — the caller
    /// creates the file, so the guard is armed *before* the create and
    /// a failed create simply finds nothing to remove.
    pub fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// The scratch path this guard owns.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The rename succeeded; there is nothing left at this path.
    pub fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            // Blocking unlink, deliberately, even on the async call
            // sites: `Drop` cannot await, and the alternative — spawning
            // a task — does not run at all when the runtime is shutting
            // down, which is the exact moment these futures get dropped.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Best-effort `fsync` of a directory.
///
/// `rename` is atomic with respect to readers, but the *directory entry*
/// it creates is itself only durable once the directory is fsynced.
/// Without this, a power loss right after the rename can leave the old
/// file (or, on some filesystems, an empty one) even though `sync_all`
/// on the temp succeeded. APFS and ext4's default `data=ordered` usually
/// paper over it; other filesystems don't.
///
/// Best-effort on purpose: some platforms (notably Windows) refuse to
/// open a directory as a file, and failing the whole write there would
/// be worse than the durability gap we're closing.
pub fn sync_dir(dir: &Path) {
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Write `contents` to `path` atomically.
///
/// Steps:
/// 1. Compute a hidden sibling temp path (see `tmp_path`).
/// 2. Write the temp file fully and sync it to disk.
/// 3. `rename(tmp, path)` — atomic on a single filesystem.
/// 4. `fsync` the parent directory so the rename itself is durable.
///
/// The temp is removed on **every** in-process exit path, not just a
/// failed rename — see [`TempFile`].
pub fn write_atomic<P: AsRef<Path>>(path: P, contents: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |file| {
        use std::io::Write;
        file.write_all(contents)
    })
}

/// [`write_atomic`] with a caller-supplied body, so a large payload can
/// be streamed into the temp instead of built in memory first.
///
/// Also the seam the failure-path tests use: a body that returns `Err`
/// exercises the same cleanup an `ENOSPC` would, without needing a full
/// filesystem.
fn write_atomic_with<P: AsRef<Path>>(
    path: P,
    write_body: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write requires a path with a parent directory",
        )
    })?;
    // Make sure the parent exists — `outl init` creates these but a
    // user moving files around could delete `pages/` and we'd hit a
    // raw IO error otherwise.
    if !parent.as_os_str().is_empty() && !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let guard = TempFile::new(tmp_path(path));

    // Write + fsync the temp. Every `?` from here to `guard.keep()`
    // drops the guard, which unlinks the scratch file.
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(guard.path())?;
        write_body(&mut file)?;
        // Flush kernel buffers to disk so a power loss between write
        // and rename can't replay garbage.
        file.sync_all()?;
    }

    fs::rename(guard.path(), path)?;
    guard.keep();

    sync_dir(parent);
    Ok(())
}

/// Read a file that is about to be **parsed, mutated, and written back**.
///
/// A missing file is the one legitimate "empty" case — a page that does
/// not exist yet renders from an empty AST. Every other error
/// (`PermissionDenied`, `Interrupted`, a raw `EIO`, an iCloud placeholder
/// whose contents have not been materialised locally yet) is propagated.
///
/// This exists because `fs::read_to_string(p).unwrap_or_default()` on a
/// rewrite path is a silent-data-loss bug: the read fails, the caller
/// parses `""` into an empty AST, renders it, and [`write_atomic`]
/// faithfully replaces a full page with nothing. The sidecar is then
/// rebuilt to match, so the hashes agree and no later scan can tell that
/// the page was ever populated. Under iCloud — where a not-yet-downloaded
/// file is exactly this kind of read failure — it is not a rare edge case.
///
/// Callers that genuinely want "absent or unreadable both mean empty"
/// must say so explicitly at the call site, and must not be on a path
/// that writes the result back.
pub fn read_for_rewrite<P: AsRef<Path>>(path: P) -> io::Result<String> {
    match fs::read_to_string(path.as_ref()) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Scratch path for an in-flight rewrite of `path`: the filename with a
/// `.tmp` suffix **and** a leading dot, so `pages/foo.md` becomes
/// `pages/.foo.md.tmp`.
///
/// The dot is the half [`TempFile`] cannot provide. A guard runs on every
/// in-process exit but not on `SIGKILL`, iOS jetsam or power loss, and an
/// undotted `pages/*.md.tmp` abandoned that way is replicated by every
/// file transport (iCloud, Syncthing, a shared FS) to every device, where
/// nothing sweeps it: `doctor --repair` prunes `snap-*.bin.tmp` and the
/// device-store scratch, and the sidecar GC only matches `*.tmp.<ulid>`.
/// Dotted, it stays local — iCloud Documents drops dotted paths outright.
///
/// A name that already starts with a dot keeps exactly one.
fn tmp_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default();
    let mut scratch = std::ffi::OsString::from(if name.as_encoded_bytes().starts_with(b".") {
        ""
    } else {
        "."
    });
    scratch.push(name);
    scratch.push(".tmp");
    path.with_file_name(scratch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn writes_and_renames() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    /// Every `*.tmp` sibling in `dir`, whatever it is called.
    ///
    /// Name-agnostic on purpose: a test that hardcodes the scratch name
    /// stops proving anything the day `tmp_path` changes, which is
    /// exactly what happened when the temp gained its leading dot.
    fn leftover_temps(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = fs::read_dir(dir)
            .expect("read temp dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "tmp"))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn no_temp_file_left_behind() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(
            leftover_temps(dir.path()),
            Vec::<std::path::PathBuf>::new(),
            "temp file should be gone after rename"
        );
    }

    /// The scratch file is hidden, because [`TempFile`] cannot run on a
    /// `SIGKILL` and an undotted `pages/*.md.tmp` left by one replicates
    /// to every device on a `transport = "file"` workspace.
    #[test]
    fn the_scratch_file_is_a_dotfile() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            tmp_path(&dir.path().join("foo.md")).file_name().unwrap(),
            ".foo.md.tmp"
        );
        // An already-hidden target keeps exactly one dot.
        assert_eq!(
            tmp_path(&dir.path().join(".peers.json"))
                .file_name()
                .unwrap(),
            ".peers.json.tmp"
        );
    }

    /// The failure path the success-only test above never reached: the
    /// body fails after the temp exists (what `ENOSPC` looks like from
    /// inside `write_all`), and the scratch must still be gone.
    #[test]
    fn temp_is_removed_when_the_write_body_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        let err = write_atomic_with(&path, |_| {
            Err(io::Error::new(io::ErrorKind::StorageFull, "no space left"))
        })
        .expect_err("a failing body must fail the write");
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
        assert_eq!(
            leftover_temps(dir.path()),
            Vec::<std::path::PathBuf>::new(),
            "a failed write body must not leak its scratch file"
        );
        assert!(!path.exists(), "a failed write must not create the target");
    }

    /// A real I/O failure, no injection: renaming onto a non-empty
    /// directory fails on every platform we ship.
    #[test]
    fn temp_is_removed_when_the_rename_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("occupant"), b"x").unwrap();

        write_atomic(&path, b"hello").expect_err("rename onto a non-empty dir must fail");
        assert_eq!(
            leftover_temps(dir.path()),
            Vec::<std::path::PathBuf>::new(),
            "a failed rename must not leak its scratch file"
        );
    }

    /// An existing file survives a failed write — the whole point of
    /// publishing by rename.
    #[test]
    fn a_failed_write_leaves_the_previous_contents_intact() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        fs::write(&path, "- original\n").unwrap();
        write_atomic_with(&path, |_| {
            Err(io::Error::new(io::ErrorKind::StorageFull, "no space left"))
        })
        .expect_err("a failing body must fail the write");
        assert_eq!(fs::read_to_string(&path).unwrap(), "- original\n");
    }

    #[test]
    fn the_guard_unlinks_an_unkept_temp() {
        let dir = TempDir::new().unwrap();
        let scratch = dir.path().join(".scratch.tmp");
        fs::write(&scratch, b"half").unwrap();
        drop(TempFile::new(scratch.clone()));
        assert!(!scratch.exists(), "dropping an armed guard must unlink");
    }

    #[test]
    fn the_guard_leaves_a_kept_temp_alone() {
        let dir = TempDir::new().unwrap();
        let scratch = dir.path().join(".scratch.tmp");
        fs::write(&scratch, b"published").unwrap();
        TempFile::new(scratch.clone()).keep();
        assert!(scratch.exists(), "keep() must disarm the unlink");
    }

    /// A guard armed over a path that was never created must not turn a
    /// missing file into an error — the create-failed path relies on it.
    #[test]
    fn the_guard_tolerates_a_temp_that_never_existed() {
        let dir = TempDir::new().unwrap();
        drop(TempFile::new(dir.path().join(".never-created.tmp")));
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        fs::write(&path, "old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn creates_missing_parent_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deep").join("nested").join("x.md");
        write_atomic(&path, b"hi").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn read_for_rewrite_returns_contents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        fs::write(&path, "- a\n").unwrap();
        assert_eq!(read_for_rewrite(&path).unwrap(), "- a\n");
    }

    #[test]
    fn read_for_rewrite_missing_file_is_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.md");
        assert_eq!(read_for_rewrite(&path).unwrap(), "");
    }

    /// The whole point: a read that fails for any reason *other* than
    /// "file isn't there" must not be reported as an empty page, because
    /// the caller is about to render that emptiness back over the file.
    #[test]
    fn read_for_rewrite_propagates_non_notfound_errors() {
        let dir = TempDir::new().unwrap();
        // A directory is readable metadata-wise but `read_to_string`
        // fails on it — a stand-in for any non-NotFound I/O failure
        // that doesn't need root or a fault-injection layer to trigger.
        let err = read_for_rewrite(dir.path()).expect_err("reading a directory must fail");
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_for_rewrite_rejects_invalid_utf8() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.md");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let err = read_for_rewrite(&path).expect_err("invalid UTF-8 must not read as empty");
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }
}
