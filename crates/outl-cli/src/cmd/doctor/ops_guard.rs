//! Keep `outl doctor` out of `ops/` — in **both** modes.
//!
//! Opening `JsonlStorage` is a read from the doctor's point of view, but
//! it is not one on disk. `JsonlStorage::open` runs `create_dir_all` and
//! then `reload`, and `reload` rebuilds the `.ops-<actor>.idx` /
//! `.ops-<actor>.nodes.idx` offset sidecars whenever they do not exactly
//! cover their `.jsonl` — **and persists them**, for every actor in the
//! directory, peers included. So a plain `outl doctor` created files
//! inside the one directory the command promises never to touch.
//!
//! Rebuilding that cache is the right behaviour for a normal boot and
//! the wrong one here. The doctor is what a user runs when they already
//! suspect the op log; sometimes over a copy they are trying to preserve
//! as evidence. "The diagnostic wrote to the thing I asked it to
//! diagnose" is not a sentence that survives a data-loss post-mortem,
//! and it is not one the `--repair` safety table should have to qualify.
//!
//! `JsonlStorage` has no read-only constructor and `outl-core` is not
//! this crate's to change, so [`OpsDirGuard`] takes the cheap route:
//! photograph `ops/` before the open, put it back byte-for-byte after.
//!
//! ## The one announced exception
//!
//! [`OpsDirGuard::capture`] takes a list of paths to **ignore**: the dead
//! index caches `--repair` has already announced it will collect (see
//! `repair::index_sidecars`). Those are the only files in `ops/` this
//! command deliberately removes, and the guard is told about them up
//! front rather than discovering the deletion afterwards and undoing it.
//!
//! Passing them in rather than forgetting them later also keeps them out
//! of RAM: `capture` reads every non-`.jsonl` file into memory so it can
//! restore it byte-for-byte, and on the workspace this feature exists for
//! that meant 134 MB of dead cache read on every `outl doctor`.
//!
//! The `.jsonl` payloads are deliberately **not** held in memory. They
//! are the one thing in there the doctor must never write at all, and on
//! the 66k-block graphs this feature exists for they are large. They are
//! watched by `(len, mtime)` instead, and a change is *reported as a
//! defect in the doctor* rather than silently papered over — restoring
//! bytes we never captured is not something this module can honestly
//! claim to do.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::Builder;

/// What one file under `ops/` looked like before the doctor ran.
enum Snapshot {
    /// A cache sidecar: small, and restorable byte-for-byte.
    Bytes(Vec<u8>),
    /// An op log: watched, never held in memory, never rewritten.
    Watched {
        /// Size in bytes at capture time.
        len: u64,
        /// Last-modified stamp, when the platform reported one.
        mtime: Option<SystemTime>,
    },
    /// Captured neither way (unreadable at capture time). Left alone.
    Opaque,
    /// Deliberately excluded: a dead cache this run announced it would
    /// collect. Never read, never restored, never removed by the guard —
    /// whatever the repair pass decides about it stands.
    Ignored,
}

/// A before-photo of `ops/`, restored by [`OpsDirGuard::restore`].
pub(super) struct OpsDirGuard {
    dir: PathBuf,
    /// Whether `ops/` existed before we opened storage. When it did not,
    /// `create_dir_all` inside `JsonlStorage::open` made it, and an
    /// empty one gets removed again.
    existed: bool,
    before: HashMap<PathBuf, Snapshot>,
}

impl OpsDirGuard {
    /// Photograph `dir` as it stands right now, skipping `ignore`.
    ///
    /// `ignore` is the set of dead cache files `--repair` has announced;
    /// see the module doc. They are recorded as [`Snapshot::Ignored`]
    /// rather than simply left out, because a path absent from the
    /// photograph is treated by [`Self::restore`] as a file that appeared
    /// mid-run and gets removed.
    pub(super) fn capture(dir: &Path, ignore: &[PathBuf]) -> Self {
        let existed = dir.is_dir();
        let mut before = HashMap::new();
        if existed {
            for path in walk(dir) {
                let snap = if ignore.contains(&path) {
                    Snapshot::Ignored
                } else if is_op_log(&path) {
                    match std::fs::metadata(&path) {
                        Ok(meta) => Snapshot::Watched {
                            len: meta.len(),
                            mtime: meta.modified().ok(),
                        },
                        Err(_) => Snapshot::Opaque,
                    }
                } else {
                    match std::fs::read(&path) {
                        Ok(bytes) => Snapshot::Bytes(bytes),
                        Err(_) => Snapshot::Opaque,
                    }
                };
                before.insert(path, snap);
            }
        }
        Self {
            dir: dir.to_path_buf(),
            existed,
            before,
        }
    }

    /// Put `ops/` back the way [`Self::capture`] found it.
    ///
    /// Succeeding silently is the whole point — a user should never see
    /// a finding about a cache file the doctor created and un-created.
    /// Only the cases we could *not* undo reach the report, and they go
    /// in as errors: a diagnostic that changed the op log is a bug in
    /// this command, and the user needs to know before they trust the
    /// rest of the run.
    pub(super) fn restore(self, b: &mut Builder) {
        let mut unrestorable: Vec<String> = Vec::new();

        for path in walk(&self.dir) {
            match self.before.get(&path) {
                None => {
                    // A file that appeared after `capture` is USUALLY an
                    // index sidecar `JsonlStorage::open` just rebuilt.
                    // But `ops/` is a sync target: iCloud, Syncthing or a
                    // co-resident client can land a brand-new
                    // `ops-<peer>.jsonl` while a 66k-block replay is
                    // still running, and this guard holds no lock.
                    //
                    // Deleting that is not "un-creating a cache file",
                    // it is destroying a peer's entire op log — and
                    // `remove_file` returning `Ok` means it would never
                    // even reach the report. Only remove what we could
                    // plausibly have made; anything else gets reported
                    // and left exactly where it is.
                    if is_op_log(&path) {
                        unrestorable.push(format!(
                            "{} appeared while the doctor was reading — another process \
                             (a peer sync, or a client sharing this workspace) is writing \
                             to the op log; left untouched",
                            path.display()
                        ));
                    } else if let Err(e) = std::fs::remove_file(&path) {
                        unrestorable.push(format!(
                            "{} was created and could not be removed: {e}",
                            path.display()
                        ));
                    }
                }
                Some(Snapshot::Bytes(bytes)) => {
                    let unchanged = std::fs::read(&path)
                        .map(|now| now == *bytes)
                        .unwrap_or(false);
                    if !unchanged {
                        if let Err(e) = std::fs::write(&path, bytes) {
                            unrestorable.push(format!(
                                "{} was modified and could not be restored: {e}",
                                path.display()
                            ));
                        }
                    }
                }
                Some(Snapshot::Watched { len, mtime }) => {
                    let meta = std::fs::metadata(&path).ok();
                    let now_len = meta.as_ref().map(|m| m.len());
                    let now_mtime = meta.and_then(|m| m.modified().ok());
                    if now_len != Some(*len) || now_mtime != *mtime {
                        // An op log that only GREW is an append by
                        // someone else — a GUI client, `outl serve`, a
                        // peer landing ops. On a synced workspace that
                        // is the common case, not the rare one, and
                        // blaming the doctor for it (an error, exit 1,
                        // "this is a bug in the doctor") teaches the
                        // user to ignore the loudest line in the report.
                        // Only a log that shrank or was rewritten is
                        // evidence something went wrong.
                        if now_len.is_some_and(|n| n > *len) {
                            b.warn(format!(
                                "{} grew while the doctor was reading it — another process \
                                 is writing to this workspace, so the findings above are a \
                                 snapshot, not a live view",
                                path.display()
                            ));
                        } else {
                            unrestorable.push(format!(
                                "{} changed during a run that must only read it",
                                path.display()
                            ));
                        }
                    }
                }
                Some(Snapshot::Opaque | Snapshot::Ignored) => {}
            }
        }

        // Anything that vanished under us. Only the sidecars we hold
        // bytes for can come back.
        for (path, snap) in &self.before {
            if path.exists() {
                continue;
            }
            match snap {
                Snapshot::Bytes(bytes) => {
                    if let Err(e) = std::fs::write(path, bytes) {
                        unrestorable.push(format!(
                            "{} was deleted and could not be restored: {e}",
                            path.display()
                        ));
                    }
                }
                // An ignored file is one this run announced it would
                // collect; its absence is the announced outcome.
                Snapshot::Ignored => {}
                _ => unrestorable.push(format!(
                    "{} was deleted during a run that must only read it",
                    path.display()
                )),
            }
        }

        if !self.existed {
            // Best-effort, and only ever on an empty dir: `remove_dir`
            // refuses a populated one, which is exactly the behaviour we
            // want if something else started writing there.
            let _ = std::fs::remove_dir(&self.dir);
        }

        for problem in unrestorable {
            b.err(format!(
                "`outl doctor` left `ops/` changed — this is a bug in the doctor, not in your \
                 workspace: {problem}"
            ));
        }
    }
}

/// Every file under `dir`, sorted. Depth mirrors `oplog::jsonl_files`
/// so the per-page layout (`ops/<actor>/<slug>.jsonl`) is covered.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir).max_depth(3) {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_file() {
            out.push(entry.path().to_path_buf());
        }
    }
    out.sort();
    out
}

fn is_op_log(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard holds no lock, and `ops/` is a sync target. A peer's
    /// op log can land mid-run: iCloud finishes a download, Syncthing
    /// delivers, or a co-resident client writes while a 66k-block
    /// replay is still going.
    ///
    /// Removing it would destroy that peer's entire history, and
    /// `remove_file` returning `Ok` means it would never even reach the
    /// report. Un-creating an index sidecar is the only thing this
    /// guard is entitled to do.
    #[test]
    fn a_peer_op_log_that_lands_mid_run_is_never_deleted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ops = tmp.path().join("ops");
        std::fs::create_dir_all(&ops).unwrap();
        std::fs::write(ops.join("ops-local.jsonl"), "{\"a\":1}\n").unwrap();

        let guard = OpsDirGuard::capture(&ops, &[]);

        // Mid-run arrivals: a peer's log (must survive) and an index
        // sidecar the storage layer rebuilt (ours to clean up).
        let peer = ops.join("ops-peer.jsonl");
        std::fs::write(&peer, "{\"b\":2}\n").unwrap();
        let sidecar = ops.join(".ops-local.idx");
        std::fs::write(&sidecar, "cache").unwrap();

        let mut b = Builder::new("ws".into(), "actor".into());
        guard.restore(&mut b);

        assert!(
            peer.exists(),
            "a peer's op log arriving mid-run must never be deleted by a diagnostic"
        );
        assert_eq!(
            std::fs::read_to_string(&peer).unwrap(),
            "{\"b\":2}\n",
            "and it must be left byte-identical, not rewritten"
        );
        assert!(
            !sidecar.exists(),
            "an index sidecar the doctor caused IS ours to remove"
        );
    }
}
