//! Which snapshots the boot cache may drop, and — the half that carries
//! the risk — which it may not.
//!
//! Nothing ever deleted a snapshot. `<root>/.outl/snapshots/` gains one
//! `snap-<actor>.bin` per actor that has ever written one on this device
//! and loses none, so a real workspace reached **54 MB in four files**,
//! one of them a schema-3 bincode body from before #207 that no build of
//! this binary will ever read again. `outl doctor --repair` was the only
//! thing that removed it, and a user has no reason to run that.
//!
//! # What a snapshot's reader actually is
//!
//! The tempting rule is "drop the snapshots of actors that are gone",
//! and it is the wrong rule twice over.
//!
//! It is **unanswerable**: an actor whose `ops-<actor>.jsonl` is not on
//! this disk right now may be a peer whose log has not been pulled yet,
//! an iCloud placeholder that has not materialized, or a file transport
//! mid-sync. That is [RFC 0211](../../../../docs/rfcs/0211-state-that-leaves-a-boundary.md)'s
//! trap — an unplugged drive and a deleted folder are the same
//! observation — and it applies here unchanged.
//!
//! It is also **irrelevant**, which is the more useful half. A snapshot
//! is never read by its actor. It is read by exactly two things:
//!
//! 1. [`crate::snapshot::read_best_from_disk`], this device's boot
//!    selector, which reads `snap-<own actor>.bin` unconditionally and
//!    otherwise ranks every candidate by its highest cutoff HLC and
//!    returns precisely one.
//! 2. `outl-sync-iroh`'s snapshot responder, which serves **only**
//!    `snap-<own actor>.bin` to a dialing peer.
//!
//! So the question that decides a snapshot's fate is not "does its
//! author still exist" but "can the selector ever choose it again", and
//! that one is answerable from the directory alone.
//!
//! # The verdicts
//!
//! - [`SnapshotVerdict::Own`] — `snap-<own actor>.bin`, and it decodes.
//!   The selector reads it before anything else and does not compare it
//!   against peers, so being behind is not evidence against it. Kept.
//! - [`SnapshotVerdict::Selected`] — the candidate the selector would
//!   adopt on a boot with no own snapshot. Kept.
//! - [`SnapshotVerdict::Superseded`] — decodes, is not ours, and another
//!   candidate outranks it under the selector's **own** comparison. Not
//!   a cleverer comparison: using one would eventually delete something
//!   the selector would have picked. Cutoffs only move forward, so no
//!   later boot brings this file back into contention. Prunable.
//! - [`SnapshotVerdict::Unusable`] — read end to end, and
//!   [`SnapshotBody::decode`] refused it. Prunable.
//! - [`SnapshotVerdict::Inconclusive`] — everything we failed to *read*
//!   rather than read and rejected, plus a body claiming a schema newer
//!   than ours. Always kept.
//!
//! # Why deletion is safe here, and why the bar is still not zero
//!
//! A snapshot is a pure cache: the op log is the source of truth, and a
//! missing snapshot costs exactly one full replay on one boot. So the
//! worst outcome of a wrong verdict here is a slow boot — far milder
//! than RFC 0211's stake, where a wrong verdict forks a workspace's write
//! actor. The caution is calibrated to that, deliberately, rather than
//! copied.
//!
//! What does *not* get relaxed is the shape of the evidence:
//!
//! - **A file we could not read is not a file we proved bad.** Same
//!   sentence `outl doctor` already says about this directory.
//! - **A newer schema is somebody's live cache.** Two builds sharing a
//!   workspace resolve to the same write actor and therefore the same
//!   `snap-<actor>.bin`, so deleting a future-schema body buys nothing
//!   and starts a delete/rewrite ping-pong between them.
//! - **A verdict is re-checked against the bytes it was computed from.**
//!   [`prune`] re-stats the file and refuses when its length or mtime
//!   moved, because a peer pull or a co-resident process can publish a
//!   fresh body between the survey and the delete. The residual hole —
//!   a replacement of identical length inside one mtime tick — costs a
//!   slow boot, which is the same price as every other wrong verdict
//!   here.
//! - **Nothing outside the snapshots directory is touched.** `parent ==`,
//!   not `starts_with`, for the reason `device/gc.rs` spells out.
//!
//! # Where it runs
//!
//! Not on a schedule and not on every boot. A boot-time sweep of this
//! directory means reading and decoding ~54 MB to reclaim disk that is
//! costing nothing yet, which is the cost root `CLAUDE.md` invariant 11
//! says to attribute before letting it decide. Instead the GC runs at
//! the two moments the evidence is already in hand:
//!
//! - **After the background writer publishes a snapshot**, on the worker
//!   thread that just serialized and fsynced a multi-MB body. That is
//!   the moment the directory *gains* a file, it is off the hot path,
//!   and it fires once per `[snapshot] threshold` ops. [`sweep`] is that
//!   entry point.
//! - **When this device's own snapshot fails to decode**, from
//!   [`crate::snapshot::read_best_from_disk`]. That is the one file the
//!   selector reads on *every* boot, so leaving it costs a full op-log
//!   replay forever, and a pure cache entry proven unreadable should not
//!   need a maintenance command to disappear.
//!
//! Two hosts it deliberately does **not** run in.
//!
//! [`crate::snapshot::read_best_from_disk`]'s directory scan is the
//! cheapest possible place — it already reads and decodes every
//! candidate, and [`survey`] *is* that pass, so the verdicts are free.
//! It still does not prune there, because opening a workspace is
//! something `outl doctor` does in its documented **read-only** mode and
//! `ops_guard.rs` only restores `ops/`. A read that silently reclaimed
//! 28 MB would break that promise for a gain the next background write
//! collects anyway. The own-snapshot drop above is the deliberate
//! exception: one file, proven dead, and `doctor` reads this directory
//! before it opens the workspace.
//!
//! The synchronous shutdown writer (`Workspace::save_snapshot`) does not
//! sweep either: it runs while a user waits for the process to exit.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{SnapshotBody, SnapshotError};
use crate::hlc::Hlc;
use crate::id::ActorId;

/// How long a leftover `snap-*.bin.tmp` is kept before it counts as
/// debris.
///
/// [`crate::snapshot::write_to_disk`] composes every snapshot in that
/// scratch file and publishes it with `rename`, so a process killed in
/// between leaves one behind and nothing has ever removed it — the same
/// "what cleans it up?" the device store's scratch files had.
///
/// A real write lives for as long as it takes to fsync a ~13 MB body, so
/// a day is several orders of magnitude of headroom.
pub const STALE_TMP_TTL: Duration = Duration::from_secs(60 * 60 * 24);

/// What the GC concluded about one `snap-*.bin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotVerdict {
    /// This device's own snapshot. The boot selector reads it first and
    /// unconditionally, and the sync transport serves it to peers.
    Own,
    /// The candidate a boot with no own snapshot would adopt.
    Selected,
    /// Decodes, but the boot selector can never choose it again.
    Superseded,
    /// Read end to end, and this binary cannot decode it.
    Unusable,
    /// Not enough was readable to say — or it was written by a newer
    /// build. **Always keeps the file.**
    Inconclusive,
}

impl SnapshotVerdict {
    /// Whether this snapshot may be removed.
    pub fn is_prunable(self) -> bool {
        matches!(self, Self::Superseded | Self::Unusable)
    }
}

/// One `snap-*.bin`, with the GC's verdict on it.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// The file's path.
    pub path: PathBuf,
    /// Whether it may be dropped, and why not when it may not.
    pub verdict: SnapshotVerdict,
    /// `(len, mtime)` as observed when the survey read the file. [`prune`]
    /// refuses when the file no longer matches, because a verdict
    /// computed against bytes that are gone is not evidence.
    stamp: Option<(u64, SystemTime)>,
    /// The directory the entry was surveyed in — the only directory
    /// [`prune`] will unlink from.
    dir: PathBuf,
}

/// Every `snap-*.bin` in one directory, judged, plus the body the boot
/// selector would adopt.
#[derive(Debug)]
pub struct Survey {
    /// One entry per `snap-*.bin`, sorted by path.
    pub entries: Vec<SnapshotEntry>,
    best: Option<SnapshotBody>,
}

impl Survey {
    /// The candidate a boot with no own snapshot would adopt, if any.
    ///
    /// This is [`crate::snapshot::read_best_from_disk`]'s directory-scan
    /// half: same ranking, same skips, computed from the same decode
    /// pass that produced the verdicts. One owner, so the selector and
    /// the GC cannot develop separate opinions about which file matters.
    pub fn into_best(self) -> Option<SnapshotBody> {
        self.best
    }

    /// The entries that may be removed.
    pub fn prunable(&self) -> impl Iterator<Item = &SnapshotEntry> {
        self.entries.iter().filter(|e| e.verdict.is_prunable())
    }

    /// Remove every prunable entry, best-effort. Returns what went.
    ///
    /// Best-effort on purpose: a snapshot is a cache, so a delete that
    /// fails costs nothing and must not abort the ones that would have
    /// succeeded — nor the boot that is calling this.
    pub fn prune_all(&self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        for entry in self.prunable() {
            match prune(entry) {
                Ok(true) => removed.push(entry.path.clone()),
                Ok(false) => {}
                Err(e) => tracing::debug!("snapshot gc: {} not removed: {e}", entry.path.display()),
            }
        }
        removed
    }
}

/// What one directory entry turned out to be, before the winner is known.
struct Candidate {
    path: PathBuf,
    name: String,
    stamp: Option<(u64, SystemTime)>,
    /// The boot selector's ranking key, when the file decoded into a body
    /// the selector would consider at all.
    key: Option<(Hlc, String)>,
    /// Read end to end and rejected by the decoder.
    dead: bool,
    /// Decoded cleanly, whatever its cutoff said.
    decoded: bool,
}

/// Judge every `snap-*.bin` in `dir` from this device's point of view.
///
/// Reads only; nothing here deletes. A missing directory is an empty
/// survey, not an error — a device that has never written a snapshot is
/// a normal state.
///
/// Exactly one decoded body is resident at a time: the running best. A
/// 2,500-page workspace's snapshot is ~13 MB, and holding every
/// candidate to compare them afterwards would spike a mobile boot by the
/// size of the directory.
pub fn survey(dir: &Path, own: ActorId) -> Result<Survey, SnapshotError> {
    let read = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Survey {
                entries: Vec::new(),
                best: None,
            })
        }
        Err(e) => {
            return Err(SnapshotError::Io(format!(
                "read dir {}: {e}",
                dir.display()
            )))
        }
    };

    let own_name = format!("snap-{own}.bin");
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut best: Option<((Hlc, String), SnapshotBody)> = None;

    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("snap-") || !name.ends_with(".bin") {
            continue;
        }
        let stamp = stamp_of(&path);
        let mut c = Candidate {
            path,
            name,
            stamp,
            key: None,
            dead: false,
            decoded: false,
        };
        // A file we cannot read says nothing about its own bytes, so it
        // stays `dead: false` and lands on `Inconclusive` below.
        if let Ok(bytes) = std::fs::read(&c.path) {
            match SnapshotBody::decode(&bytes) {
                Ok(body) => {
                    c.decoded = true;
                    c.key = selection_key(&body, &c.name);
                    // The device's own snapshot is read first and
                    // unconditionally by the selector, so it never
                    // competes for the adopt slot.
                    if c.name != own_name {
                        if let Some(key) = c.key.clone() {
                            if best.as_ref().is_none_or(|(b, _)| key > *b) {
                                best = Some((key, body));
                            }
                        }
                    }
                }
                Err(e) => c.dead = proves_dead(&e),
            }
        }
        candidates.push(c);
    }

    let winner = best.as_ref().map(|(k, _)| k.clone());
    let mut entries: Vec<SnapshotEntry> = candidates
        .into_iter()
        .map(|c| {
            let verdict = if c.name == own_name {
                // Own and undecodable is the expensive case: the
                // selector reads it on every boot and falls back to a
                // full replay every time.
                if c.dead {
                    SnapshotVerdict::Unusable
                } else if c.decoded {
                    SnapshotVerdict::Own
                } else {
                    SnapshotVerdict::Inconclusive
                }
            } else if c.dead {
                SnapshotVerdict::Unusable
            } else if c.decoded {
                // Ranked by the selector's own comparison, so the GC
                // cannot drop a file the selector would have picked.
                if c.key.is_some() && c.key == winner {
                    SnapshotVerdict::Selected
                } else {
                    SnapshotVerdict::Superseded
                }
            } else {
                SnapshotVerdict::Inconclusive
            };
            SnapshotEntry {
                path: c.path,
                verdict,
                stamp: c.stamp,
                dir: dir.to_path_buf(),
            }
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Survey {
        entries,
        best: best.map(|(_, body)| body),
    })
}

/// Delete one surveyed snapshot, after re-checking that the bytes the
/// verdict was computed from are still the bytes on disk.
///
/// The re-check is not ceremony. A peer pull publishes into this very
/// directory with a `rename`, and a co-resident process writes its own
/// snapshot here too, so between the survey and the delete a path can
/// come to name a body that is current and wanted.
///
/// Returns whether anything was removed. An already-absent file is
/// `Ok(false)`, not an error.
pub fn prune(entry: &SnapshotEntry) -> Result<bool, SnapshotError> {
    if !entry.verdict.is_prunable() {
        return Ok(false);
    }
    // `parent ==`, not `starts_with`: `Path::starts_with` compares
    // components without normalising, so `<dir>/../../x` passes it. Every
    // real snapshot sits directly in the directory it was surveyed in.
    if entry.path.parent() != Some(entry.dir.as_path()) {
        return Ok(false);
    }
    let Some(stamp) = entry.stamp else {
        return Ok(false);
    };
    if stamp_of(&entry.path) != Some(stamp) {
        return Ok(false);
    }
    remove(&entry.path)
}

/// Leftover `snap-*.bin.tmp` scratch files older than `ttl`.
///
/// Read-only, like [`survey`]; [`prune_tmp`] is what acts. These are
/// deliberately kept out of the snapshot listing: a scratch file is a
/// write that never became a snapshot, so reporting one as a snapshot
/// whose reader is gone would invent a cache entry that never existed.
pub fn stale_tmp(dir: &Path, ttl: Duration) -> Result<Vec<PathBuf>, SnapshotError> {
    let read = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(SnapshotError::Io(format!(
                "read dir {}: {e}",
                dir.display()
            )))
        }
    };
    let now = SystemTime::now();
    let mut out: Vec<PathBuf> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| is_scratch(p) && older_than(p, now, ttl))
        .collect();
    out.sort();
    Ok(out)
}

/// Delete one abandoned scratch file, after re-checking that it is still
/// stale and still inside a snapshots directory.
pub fn prune_tmp(path: &Path, ttl: Duration) -> Result<bool, SnapshotError> {
    // No survey to carry the directory, so the directory has to identify
    // itself. `<root>/.outl/snapshots` is the only place outl writes one
    // of these, and a `remove_file` driven by a filename pattern alone is
    // how a GC reaches somewhere it was never meant to.
    let in_snapshots_dir = path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "snapshots");
    if !in_snapshots_dir || !is_scratch(path) || !older_than(path, SystemTime::now(), ttl) {
        return Ok(false);
    }
    remove(path)
}

/// Survey `dir`, drop everything prunable, and drop the scratch files a
/// killed writer abandoned. Returns the paths removed, sorted.
pub fn sweep(dir: &Path, own: ActorId) -> Result<Vec<PathBuf>, SnapshotError> {
    let mut removed = survey(dir, own)?.prune_all();
    for path in stale_tmp(dir, STALE_TMP_TTL)? {
        match prune_tmp(&path, STALE_TMP_TTL) {
            Ok(true) => removed.push(path),
            Ok(false) => {}
            Err(e) => tracing::debug!("snapshot gc: {} not removed: {e}", path.display()),
        }
    }
    removed.sort();
    Ok(removed)
}

/// Drop `snap-<own>.bin` when the boot selector just read it end to end
/// and [`SnapshotBody::decode`] refused it.
///
/// This is the one deletion worth doing on a boot that is otherwise
/// touching nothing else in the directory: the selector reads this exact
/// file first on *every* boot, so leaving it makes that device replay the
/// whole op log forever, and no user has a reason to run a maintenance
/// command over a cache.
pub(crate) fn drop_own_if_unusable(dir: &Path, own: ActorId, err: &SnapshotError) {
    if !proves_dead(err) {
        return;
    }
    let path = dir.join(format!("snap-{own}.bin"));
    // Re-read rather than trusting the caller's error: the same two-pass
    // re-check [`prune`] makes, for the same reason.
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let still_dead = match SnapshotBody::decode(&bytes) {
        Ok(_) => false,
        Err(e) => proves_dead(&e),
    };
    if !still_dead {
        return;
    }
    match remove(&path) {
        Ok(true) => tracing::debug!(
            "snapshot gc: dropped unreadable own snapshot {}",
            path.display()
        ),
        Ok(false) => {}
        Err(e) => tracing::debug!("snapshot gc: {} not removed: {e}", path.display()),
    }
}

fn remove(path: &Path) -> Result<bool, SnapshotError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(SnapshotError::Io(format!("remove {}: {e}", path.display()))),
    }
}

/// Whether `path` is one of [`crate::snapshot::write_to_disk`]'s in-flight
/// scratch files rather than a published snapshot.
fn is_scratch(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("snap-") && n.ends_with(".bin.tmp"))
}

fn older_than(path: &Path, now: SystemTime, ttl: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age >= ttl)
}

/// `(len, mtime)` for a path, where the platform will give both.
///
/// `None` keeps the file: [`prune`] has nothing to re-check against, and
/// a delete on unverifiable evidence is the thing this module refuses.
fn stamp_of(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Whether this error is the decoder saying "I read these bytes and they
/// are not a snapshot I can ever use".
///
/// [`SnapshotError::Io`] is not: we never saw the bytes. A
/// [`SnapshotError::SchemaMismatch`] naming a version *above* ours is not
/// either — it was written by a newer build sharing this workspace, and
/// two builds on one device resolve to the same write actor and therefore
/// the same `snap-<actor>.bin`, so deleting it buys nothing and starts a
/// delete/rewrite ping-pong.
fn proves_dead(err: &SnapshotError) -> bool {
    match err {
        SnapshotError::Decode(_) | SnapshotError::HashMismatch => true,
        SnapshotError::SchemaMismatch { expected, found } => found < expected,
        SnapshotError::Encode(_) | SnapshotError::Io(_) => false,
    }
}

/// The boot selector's ranking key for a body: highest cutoff HLC, then
/// filename. A body with an empty cutoff has no key — the selector skips
/// it outright, since adopting it would save no replay.
fn selection_key(body: &SnapshotBody, name: &str) -> Option<(Hlc, String)> {
    body.cutoff
        .values()
        .copied()
        .max()
        .map(|hlc| (hlc, name.to_string()))
}

#[cfg(test)]
mod tests;
