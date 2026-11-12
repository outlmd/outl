//! The lock that serializes "replace a sidecar" against "append to it".
//!
//! The live writer appends one entry per op to the offset and node sidecars
//! by path. A second opener of the same workspace that finds those sidecars
//! behind the `.jsonl` indexes the tail in memory and then renames a whole new
//! file over each. Anything the writer appended to the OLD inode between the
//! opener's read and its rename is gone, while every later append lands in
//! the new file and keeps the max-offset record valid. The freshness ladder
//! then accepts a sidecar with a hole in the middle as byte-exact, forever.
//!
//! Both sides take this flock exclusively: the writer around one batch
//! (jsonl write + sidecar appends), a persister around "index whatever tail
//! appeared, then rename". Under it the persister sees a quiescent file, so
//! what it renames covers all of it.
//!
//! Placement follows `ops/.append.lock` and `ops/.lock-<actor>`: a dotfile
//! next to the sidecars, empty, recreated on demand. flock state is
//! kernel-local, so losing the file in a sync costs nothing.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Held for as long as the value lives; the closed fd releases it.
pub(super) struct IdxLock {
    _file: File,
}

/// `.idx-lock-<stem>` next to `idx_path` (the offset sidecar). The name is
/// derived from the sidecar's so each `(actor, scope)` has its own lock, and
/// it does not end in `.idx`, so sidecar GC never mistakes it for one.
fn lock_path(idx_path: &Path) -> PathBuf {
    let name = idx_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let stem = name.trim_start_matches('.').trim_end_matches(".idx");
    idx_path.with_file_name(format!(".idx-lock-{stem}"))
}

impl IdxLock {
    /// Block until the lock for `idx_path`'s sidecar pair is ours.
    ///
    /// Uses `fs2` rather than `File::lock`, which is `Unsupported` on
    /// Android. An error means the lock could not be had (read-only
    /// directory, say); callers treat the sidecars as best-effort caches and
    /// must not persist without it.
    pub(super) fn acquire(idx_path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path(idx_path))?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}
