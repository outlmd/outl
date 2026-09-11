//! Which index sidecars `ops/` may drop, and — the half that carries the
//! risk — which it may not.
//!
//! Nothing ever deleted one. A real 2,560-page workspace held, next to
//! 20 op logs:
//!
//! | shape | files | size | read by |
//! |---|---|---|---|
//! | `.ops-<actor>.idx`, `.ops-<actor>.nodes.idx` | 40 | 51 MB | the current code |
//! | `ops-<actor>.idx`, `ops-<actor>.nodes.idx` | 40 | 50 MB | **nothing** |
//! | `ops-<actor>.idx.tmp.<ulid>` | 16 | 84 MB | **nothing** |
//!
//! 134 MB of dead cache — and worse than dead, because `ops/` is
//! deliberately **not** a dotfile so that it syncs, which means the
//! undotted half rode the file transport to every other device. The
//! sidecars were moved to dot-prefixed names precisely so they would stop
//! doing that; nothing removed what the rename left behind.
//!
//! # What makes a sidecar dead
//!
//! An index maps HLCs (and nodes) to **byte offsets** inside one
//! `.jsonl`. Its only reader is the boot path, which composes the name it
//! wants from [`super::file_name`]. So the question is not "does this
//! actor still exist" — `ops/` is full of peers whose logs have not been
//! pulled yet, and [RFC 0211](../../../../../docs/rfcs/0211-state-that-leaves-a-boundary.md)'s
//! trap applies unchanged: an unplugged drive and a deleted device look
//! identical. The question is **"can the reader ever compose this
//! name again"**, and that one is answerable from the filename alone.
//!
//! # The verdicts
//!
//! - [`SidecarVerdict::Live`] — the name [`super::file_name`] composes
//!   today. Kept, regardless of whether its `.jsonl` is on this disk.
//! - [`SidecarVerdict::Legacy`] — the same name without its leading dot.
//!   Nothing composes it, so nothing reads it. Prunable.
//! - [`SidecarVerdict::AbandonedScratch`] — a `*.tmp.<ulid>` older than
//!   [`STALE_TMP_TTL`]: a write that never renamed. Prunable.
//! - [`SidecarVerdict::Inconclusive`] — a temp that may still be in
//!   flight, a file we could not stat, or a directory where the live and
//!   legacy spellings have become the same string. **Always kept.**
//!
//! Everything else in `ops/` is not surveyed at all. A `.jsonl` never
//! enters (it is the source of truth, and a peer's is normal), a
//! `.lock-<actor>` never enters (see below), and neither does any name
//! this module cannot attribute to an actor and a kind — *a file we
//! cannot account for is not a file we proved dead.*
//!
//! # Why `.lock-<actor>` is not here
//!
//! A real workspace also carried 23 orphaned `.lock-<actor>` files, and
//! they stay. A lock is **not** a cache. `ActorWriteLock` flocks that
//! path; the file's *existence* is not the lock, so deleting one while a
//! process holds it lets the next process create a fresh inode, flock it
//! successfully, and believe it owns the same actor — two writers
//! appending to one `ops-<actor>.jsonl`, which is the interleaved-append
//! corruption the read path already has to recover from.
//!
//! And the cost side does not argue for it either (root `CLAUDE.md`
//! invariant 11): every one of those files is **0 bytes**. The reclaim is
//! a directory entry. Trading an arbitration failure for that is not a
//! trade. `outl doctor` reports the count instead, because a workspace
//! with far more lock files than op logs is telling you something real —
//! how many ephemeral actors it has minted.
//!
//! # Why deletion is safe here, and why the bar is still not zero
//!
//! An index is a pure cache rebuilt from the `.jsonl` sitting right next
//! to it, so the worst outcome of a wrong verdict is **one slower boot**.
//! That is milder than RFC 0211's stake (a wrong verdict there forks a
//! workspace's write actor) and milder than
//! [RFC 0258](../../../../../docs/rfcs/0258-snapshot-cache-lifecycle.md)'s,
//! and the caution here is calibrated to it rather than inherited.
//!
//! What does **not** relax is the shape of the evidence:
//!
//! - **A file we could not read is not a file we proved dead.** No
//!   `(len, mtime)` stamp means [`prune`] refuses.
//! - **A verdict is re-checked against the bytes it was computed from.**
//!   `ops/` is a sync target; a file can be replaced between the survey
//!   and the unlink.
//! - **Nothing outside the surveyed directory is touched.** `parent ==`,
//!   not `starts_with`, for the reason `device/gc.rs` spells out.
//! - **The rule disarms itself.** The entire basis for calling
//!   `ops-<actor>.idx` dead is that [`super::file_name`] emits a dot. If
//!   that ever stops being true, the undotted file becomes the *live*
//!   cache — so `legacy_name` is derived from the live name, and a
//!   directory where the two spellings coincide yields
//!   [`SidecarVerdict::Inconclusive`] for everything.
//!
//! # Scope
//!
//! Only the directory it is handed, one level, files only. Per-page shard
//! directories (`ops/<actor>/`) are deliberately **not** descended into:
//! deciding which `.<slug>.idx` in there is live needs that directory's
//! `.jsonl` set, and a wrong call deletes a live cache to reclaim a few
//! KB. See `docs/rfcs/0265-index-sidecar-lifecycle.md`.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::id::ActorId;
use crate::storage::sidecar::{file_name, SidecarKind};
use crate::storage::{PageScope, StorageError};

/// How long a leftover `*.tmp.<ulid>` is kept before it counts as debris.
///
/// Deliberately the **same constant** the snapshot cache uses, not a
/// second answer to the same question: both are "a scratch file whose
/// publishing rename may still be seconds away", and a real write lives
/// for as long as one fsync.
pub use crate::snapshot::gc::STALE_TMP_TTL;

/// What the GC concluded about one file in an ops directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarVerdict {
    /// The name the reader composes today. Kept.
    Live,
    /// The pre-dotfile spelling of a live name. Nothing reads it, and
    /// while it sat undotted in `ops/` it rode the file-sync transport.
    Legacy,
    /// A write temp older than [`STALE_TMP_TTL`] — a write that never
    /// published.
    AbandonedScratch,
    /// Not enough to say. **Always keeps the file.**
    Inconclusive,
}

impl SidecarVerdict {
    /// Whether this file may be removed.
    pub fn is_prunable(self) -> bool {
        matches!(self, Self::Legacy | Self::AbandonedScratch)
    }

    /// One sentence a user-facing report can print verbatim.
    ///
    /// Owned here rather than by each surface, so a second surface cannot
    /// invent a different explanation of the same deletion.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Live => "the index the next boot will read",
            Self::Legacy => {
                "pre-dotfile index sidecar — nothing composes this name any more, so nothing \
                 reads it, and while it sat undotted in `ops/` it synced to every other device"
            }
            Self::AbandonedScratch => {
                "abandoned index write — a process was killed between creating the temp and \
                 renaming it into place, so it never became an index"
            }
            Self::Inconclusive => "kept — not enough evidence to call it dead",
        }
    }
}

/// One surveyed file, with the GC's verdict on it.
#[derive(Debug, Clone)]
pub struct SidecarEntry {
    /// The file.
    pub path: PathBuf,
    /// Whether it may go, and why not when it may not.
    pub verdict: SidecarVerdict,
    /// Size when it was surveyed, so a report can say what is reclaimed.
    pub bytes: u64,
    actor: ActorId,
    kind: SidecarKind,
    /// `(len, mtime)` as observed by [`survey`]. [`prune`] refuses when
    /// the file no longer matches — a verdict computed against bytes that
    /// are gone is not evidence.
    stamp: Option<(u64, SystemTime)>,
    /// The only directory [`prune`] will unlink from.
    dir: PathBuf,
}

impl SidecarEntry {
    /// The actor whose cache this is.
    pub fn actor(&self) -> Option<ActorId> {
        Some(self.actor)
    }

    /// Which of the two caches it is.
    pub fn kind(&self) -> SidecarKind {
        self.kind
    }
}

/// Every sidecar-shaped file in one directory, judged.
#[derive(Debug, Default)]
pub struct Survey {
    /// One entry per surveyed file, sorted by path.
    pub entries: Vec<SidecarEntry>,
}

impl Survey {
    /// The entries that may be removed.
    pub fn prunable(&self) -> impl Iterator<Item = &SidecarEntry> {
        self.entries.iter().filter(|e| e.verdict.is_prunable())
    }

    /// Bytes the prunable entries were holding when surveyed.
    pub fn reclaimable_bytes(&self) -> u64 {
        self.prunable().map(|e| e.bytes).sum()
    }

    /// Remove every prunable entry, best-effort. Returns what went.
    ///
    /// Best-effort on purpose: these are caches, so a delete that fails
    /// costs nothing and must not abort the ones that would have
    /// succeeded.
    pub fn prune_all(&self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        for entry in self.prunable() {
            match prune(entry) {
                Ok(true) => removed.push(entry.path.clone()),
                Ok(false) => {}
                Err(e) => tracing::debug!("sidecar gc: {} not removed: {e}", entry.path.display()),
            }
        }
        removed.sort();
        removed
    }
}

/// The undotted spelling of a live sidecar name.
///
/// **Derived, never typed out.** [`classify`] calls a file dead by
/// comparing it against this, so if [`file_name`] ever stopped emitting a
/// dot, a hand-written copy here would keep naming the file that has just
/// become live. Derivation makes the two collapse into one string
/// instead, which [`classify`] detects and refuses on.
pub(crate) fn legacy_name(actor: ActorId, kind: SidecarKind) -> String {
    file_name(actor, &PageScope::Global, kind)
        .trim_start_matches('.')
        .to_string()
}

/// Which generation a surveyed name belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generation {
    Live,
    Legacy,
    /// The live and legacy spellings have become the same string, so the
    /// name proves nothing either way.
    Ambiguous,
}

/// A filename this module recognises.
struct Candidate {
    actor: ActorId,
    kind: SidecarKind,
    generation: Generation,
    /// Whether it carries a `.tmp.<ulid>` suffix — a write in progress,
    /// or one that never finished.
    scratch: bool,
}

/// Attribute a filename to `(actor, kind, generation)`, or refuse.
///
/// `None` means "not a file this module produced", and the caller drops
/// it from the survey entirely rather than recording an opinion about it.
fn classify(name: &str) -> Option<Candidate> {
    // The op log is never a candidate, in any spelling — including the
    // conflict copies file-sync tools leave (`ops-<a> 2.jsonl`,
    // `ops-<a>.jsonl.sync-conflict-…`).
    if name.contains(".jsonl") {
        return None;
    }
    let (base, scratch) = match name.rsplit_once(".tmp.") {
        // A `.tmp.` whose suffix is not a ULID was not written by
        // `write_atomic`.
        Some((base, suffix)) if ulid::Ulid::from_string(suffix).is_ok() => (base, true),
        Some(_) => return None,
        None => (name, false),
    };

    // Longest suffix first: `.nodes.idx` also ends with `.idx`.
    for kind in [SidecarKind::Node, SidecarKind::Offset] {
        let Some(stem) = base.strip_suffix(kind.suffix()) else {
            continue;
        };
        let bare = stem.strip_prefix('.').unwrap_or(stem);
        let actor = bare
            .strip_prefix("ops-")
            .and_then(|s| ulid::Ulid::from_string(s).ok())
            .map(ActorId)?;

        let live = file_name(actor, &PageScope::Global, kind);
        let legacy = legacy_name(actor, kind);
        let generation = if live == legacy {
            Generation::Ambiguous
        } else if base == live {
            Generation::Live
        } else if base == legacy {
            Generation::Legacy
        } else {
            return None;
        };
        return Some(Candidate {
            actor,
            kind,
            generation,
            scratch,
        });
    }
    None
}

/// Judge every sidecar-shaped file directly inside `dir`.
///
/// Reads names and metadata only; nothing here deletes, and nothing here
/// opens a file. A missing directory is an empty survey, not an error — a
/// workspace that has never written an op is a normal state.
pub fn survey(dir: &Path) -> Result<Survey, StorageError> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Survey::default()),
        Err(e) => {
            return Err(StorageError::Backend(format!(
                "read dir {}: {e}",
                dir.display()
            )))
        }
    };
    let now = SystemTime::now();
    let mut entries = Vec::new();
    for entry in read.flatten() {
        // Files only. A per-page shard directory is not descended into
        // (see the module doc).
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(candidate) = classify(name) else {
            continue;
        };
        let stamp = stamp_of(&path);
        let verdict = judge(&candidate, stamp, now);
        entries.push(SidecarEntry {
            bytes: stamp.map(|(len, _)| len).unwrap_or(0),
            path,
            verdict,
            actor: candidate.actor,
            kind: candidate.kind,
            stamp,
            dir: dir.to_path_buf(),
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Survey { entries })
}

/// The verdict for one attributed file.
fn judge(
    candidate: &Candidate,
    stamp: Option<(u64, SystemTime)>,
    now: SystemTime,
) -> SidecarVerdict {
    if candidate.generation == Generation::Ambiguous {
        return SidecarVerdict::Inconclusive;
    }
    if candidate.scratch {
        // A temp is judged on age alone: its name says nothing about
        // whether the write that made it is still running.
        let old_enough = stamp
            .and_then(|(_, mtime)| now.duration_since(mtime).ok())
            .is_some_and(|age| age >= STALE_TMP_TTL);
        return if old_enough {
            SidecarVerdict::AbandonedScratch
        } else {
            SidecarVerdict::Inconclusive
        };
    }
    match candidate.generation {
        Generation::Live => SidecarVerdict::Live,
        Generation::Legacy => SidecarVerdict::Legacy,
        Generation::Ambiguous => SidecarVerdict::Inconclusive,
    }
}

/// Delete one surveyed file, after re-checking that the bytes the verdict
/// was computed from are still the bytes on disk.
///
/// Returns whether anything was removed. An already-absent file is
/// `Ok(false)`, not an error.
pub fn prune(entry: &SidecarEntry) -> Result<bool, StorageError> {
    if !entry.verdict.is_prunable() {
        return Ok(false);
    }
    // `parent ==`, not `starts_with`: `Path::starts_with` compares
    // components without normalising, so `<dir>/../../x` passes it.
    if entry.path.parent() != Some(entry.dir.as_path()) {
        return Ok(false);
    }
    let Some(stamp) = entry.stamp else {
        return Ok(false);
    };
    if stamp_of(&entry.path) != Some(stamp) {
        return Ok(false);
    }
    match std::fs::remove_file(&entry.path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(StorageError::Backend(format!(
            "remove {}: {e}",
            entry.path.display()
        ))),
    }
}

/// Survey `dir` and drop everything prunable. Returns the paths removed.
pub fn sweep(dir: &Path) -> Result<Vec<PathBuf>, StorageError> {
    Ok(survey(dir)?.prune_all())
}

/// `(len, mtime)` for a path, where the platform will give both.
///
/// `None` keeps the file: [`prune`] has nothing to re-check against, and
/// a delete on unverifiable evidence is the thing this module refuses.
fn stamp_of(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}
