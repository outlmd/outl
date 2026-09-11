//! Op-log compaction: drop ops that are provably inert, keep everything else.
//!
//! The op log is the source of truth (root `CLAUDE.md` invariant 1) and
//! rewriting `ops/*.jsonl` is the most destructive thing this codebase
//! can do. This module exists anyway because the log also grows forever:
//! it replays on every boot and ships whole to every newly paired device
//! ([issue #110](https://github.com/outlmd/outl/issues/110)).
//!
//! # What it drops, and why that is safe
//!
//! One shape only: a [`Op::Move`] that restates the placement its own
//! [`Op::Create`] made immediately before it.
//!
//! ```text
//! {"ts":…,"op":{"Create":{"node":N,"parent":P,"position":X}}}
//! {"ts":…,"op":{"Move":{"node":N,"new_parent":P,"position":X,…}}}
//! ```
//!
//! `outl-md`'s `diff.rs` emits that pair defensively; `reconcile.rs` now
//! filters the redundant half before it reaches disk, so this is dead
//! weight already written, not a bug still being produced. On the
//! reference workspace it is 62,209 ops / 18.3 MB — 22.6% of the log.
//!
//! **The trap this module is built around:** `Op::Create` is idempotent,
//! so a `Create` for a node that already exists does nothing and the
//! `Move` after it is the op doing the work. The sharpest instance is a
//! block that was deleted (`Move(n, TRASH_ROOT)`) and later restored —
//! there the `Create` is a no-op and dropping the `Move` deletes the
//! user's block with no trace. The adjacent pair alone can never tell
//! those two cases apart; see [`plan_compaction`] for the predicate that can, and
//! [RFC 0256](../../../../docs/rfcs/0256-op-log-compaction.md) for the
//! soundness argument, the rejected alternatives and the residual risk.
//!
//! # Why dropping historical ops cannot break undo
//!
//! The resident undo stack works off the in-memory log of the *running*
//! session: a client pushes an entry when it applies an op it just
//! authored, and undo emits a **new, compensating op**. It never
//! re-reads a historical line, and it never reverses an op it did not
//! author this session. Compaction refuses to run at all while any
//! process holds the workspace (see [`apply_compaction`]), so
//! there is no session whose stack could name a dropped op. And the ops
//! it drops are, by the predicate, ones whose application changes
//! nothing — so even a hypothetical historical undo would be undoing a
//! no-op.
//!
//! [`Op::Move`]: crate::op::Op::Move
//! [`Op::Create`]: crate::op::Op::Create

mod plan;
mod rewrite;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use thiserror::Error;

use crate::id::ActorId;
use crate::storage::StorageError;

pub use plan::{plan_compaction, CompactOptions, CompactPlan, DEFAULT_HORIZON_MS};
pub use rewrite::{apply_compaction, apply_compaction_as};

/// What compaction would remove from (or removed from) one actor's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorSaving {
    /// The actor whose `ops-<actor>.jsonl` this describes.
    pub actor: ActorId,
    /// Ops the file holds today.
    pub ops_total: usize,
    /// Ops the plan drops.
    pub ops_dropped: usize,
    /// Bytes the file holds today.
    pub bytes_total: u64,
    /// Bytes the dropped lines account for.
    pub bytes_dropped: u64,
}

impl ActorSaving {
    /// Fraction of this file's bytes the plan removes, 0.0 when empty.
    pub fn percent(&self) -> f64 {
        if self.bytes_total == 0 {
            0.0
        } else {
            100.0 * self.bytes_dropped as f64 / self.bytes_total as f64
        }
    }
}

/// What a compaction pass would do, or did.
///
/// Produced by [`plan_compaction`] (nothing written) and returned again
/// by [`apply_compaction`] (with [`Self::backup_dir`] filled in).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompactReport {
    /// Ops across every actor file.
    pub ops_total: usize,
    /// Ops the plan drops.
    pub ops_dropped: usize,
    /// Bytes across every actor file.
    pub bytes_total: u64,
    /// Bytes the dropped lines account for.
    pub bytes_dropped: u64,
    /// Per-actor breakdown, sorted by actor id.
    pub actors: Vec<ActorSaving>,
    /// Where the pre-compaction files were copied. `None` for a plan
    /// that was never applied, and for an applied plan that had nothing
    /// to drop.
    pub backup_dir: Option<PathBuf>,
}

impl CompactReport {
    /// Fraction of the log's bytes the plan removes, 0.0 when empty.
    pub fn percent(&self) -> f64 {
        if self.bytes_total == 0 {
            0.0
        } else {
            100.0 * self.bytes_dropped as f64 / self.bytes_total as f64
        }
    }
}

/// Why a compaction pass refused to run.
///
/// Every variant is a refusal, never a partial rewrite: the whole point
/// is that `ops/` is either untouched or replaced atomically per file.
#[derive(Debug, Error)]
pub enum CompactError {
    /// Underlying storage error while reading the log.
    #[error("storage: {0}")]
    Storage(#[from] StorageError),

    /// I/O failure on a specific path.
    #[error("io error on {path}: {source}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// The workspace is open somewhere else. Compaction rewrites files a
    /// live process holds byte offsets into, so it never races one.
    #[error("the workspace is open in another outl process ({0}) — close it and retry")]
    Busy(PathBuf),

    /// A record in the log did not parse. A damaged log is *reported*,
    /// never rewritten — we cannot know what the lost op said, and the
    /// rewrite would make the loss permanent.
    #[error("{path}: line {line} did not parse ({reason}) — run `outl doctor` before compacting; a damaged log is never rewritten")]
    DamagedLog {
        /// File holding the bad record.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Parser's complaint.
        reason: String,
    },

    /// The log changed between planning and applying.
    #[error("{0} changed on disk since the plan was made — re-run the plan")]
    PlanStale(PathBuf),

    /// The plan would rewrite an `ops-<actor>.jsonl` this device does not
    /// own. See [`apply_compaction_as`].
    #[error(
        "ops-{actor}.jsonl belongs to another device (this one writes as {own}) — shortening it \
         publishes a competing, shorter version of that path, and every file transport (iCloud, \
         Syncthing, shared FS) resolves that last-write-wins, so the owning device's unshipped \
         ops die with it. Run `outl compact --apply` on that device, or `--force` here if you \
         are certain no file transport carries this workspace"
    )]
    ForeignActorFile {
        /// The file the plan wanted to rewrite.
        actor: ActorId,
        /// The actor this device writes under.
        own: ActorId,
    },

    /// A rewrite failed *after* the backup was taken. Carries the backup
    /// directory, because that is the one moment its path is needed.
    #[error("the rewrite failed after the backup was taken, so `ops/` may be partly rewritten — the pre-compaction op log is intact at {backup_dir}; restore it with `cp {backup_dir}/*.jsonl <workspace>/ops/`", backup_dir = backup_dir.display())]
    RewriteFailed {
        /// Where the pre-compaction files were copied.
        backup_dir: PathBuf,
        /// What went wrong.
        #[source]
        source: Box<CompactError>,
    },

    /// The per-page op-log layout (RFC #137 Phase B) is not supported.
    /// Refusing is the point: compacting only the `Global` half of a
    /// mixed workspace would decide inertness against an incomplete log.
    #[error("{0} uses the per-page op-log layout, which compaction does not support")]
    PerPageLayout(PathBuf),
}
