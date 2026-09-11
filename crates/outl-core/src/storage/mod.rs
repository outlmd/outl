//! Storage abstraction.
//!
//! The op log is the source of truth; storage is how we persist it. The
//! `Storage` trait is the single boundary between `outl-core` and any
//! particular persistence layer. See `docs/storage.md`.

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::LogOp;
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

pub mod compact;
pub mod index;
pub mod jsonl;
pub mod memory;
pub mod node_index;
pub mod sidecar;

pub use compact::{
    apply_compaction, plan_compaction, CompactError, CompactOptions, CompactPlan, CompactReport,
};
pub use index::{ActorIndex, OffsetIndex};
pub use jsonl::JsonlStorage;
pub use memory::MemoryStorage;
pub use node_index::{ActorNodeIndex, NodeIndex};
pub use sidecar::SidecarKind;

/// Which page a storage backend is responsible for.
///
/// `Global` is the legacy single-file-per-actor layout every workspace
/// shipped with before RFC #137 Phase B. `PerPage(slug)` is the
/// sharded layout — one `ops/<actor>/<slug>.jsonl` per (actor, page)
/// pair — that lets boot and sync be proportional to the active page
/// rather than the whole workspace.
///
/// Layouts on disk:
///
/// - `Global` → `ops/ops-<actor>.jsonl` (+ `.idx`, `.nodes.idx` sidecars)
/// - `PerPage(slug)` → `ops/<actor>/<slug>.jsonl` (+ sidecars)
///
/// `Global` stays the default for back-compat. New workspaces opt into
/// `PerPage` via `outl init --scope=per-page`; existing ones migrate
/// via `outl migrate-to-per-page-ops`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PageScope {
    /// Single op log per actor — every page shares one file.
    #[default]
    Global,
    /// Op log scoped to one page. The slug is the page's URL-safe name
    /// (same one used for the `.md` filename under `pages/`).
    PerPage(String),
}

impl PageScope {
    /// `true` for the legacy single-file layout.
    pub fn is_global(&self) -> bool {
        matches!(self, PageScope::Global)
    }
}

/// Errors a `Storage` implementation may produce.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Underlying I/O or backend error.
    #[error("storage backend error: {0}")]
    Backend(String),

    /// Serialization failure for op or snapshot data.
    #[error("serialization error: {0}")]
    Serialize(String),

    /// Integrity check failed (e.g. SQLite `integrity_check`).
    #[error("integrity error: {0}")]
    Integrity(String),

    /// Op log corruption: an op that should be present is missing.
    #[error("missing op: {0}")]
    MissingOp(String),
}

/// Storage backend trait.
///
/// Implementations: [`JsonlStorage`] (one file per actor, the only
/// persistent backend), [`MemoryStorage`] (test double, no disk),
/// future `ChronDbStorage` (issue #1).
pub trait Storage: Send + Sync {
    /// Append a new op. Must be durable before returning `Ok`.
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError>;

    /// Append a batch of ops. Durable before returning `Ok`.
    ///
    /// The whole batch is durable on `Ok`; the tail is "atomic-ish" — a
    /// crash mid-batch can leave a prefix of the ops persisted with the
    /// last line possibly torn, which the read path's torn-tail self-heal
    /// recovers on the next append (see [`JsonlStorage`]). No op past the
    /// last durable line survives.
    ///
    /// Default impl loops [`Self::append_op`] (one fsync each). Backends
    /// should override to amortize the durability cost to one fsync per
    /// batch — that is the whole point of the API.
    fn append_ops(&mut self, ops: &[LogOp]) -> Result<(), StorageError> {
        ops.iter().try_for_each(|op| self.append_op(op))
    }

    /// Return all ops with HLC strictly greater than `ts`, in HLC order.
    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops touching the given node.
    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops created by the given actor.
    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError>;

    /// Return the most recent HLC seen per actor (used for sync vector clocks).
    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError>;

    /// Return all ops in HLC order.
    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError>;

    /// Return every op that a snapshot with the given per-actor `cutoff`
    /// has **not** yet folded into its materialized state, in HLC order.
    ///
    /// An op qualifies when its HLC is strictly greater than the cutoff
    /// recorded for **its own actor**, or when its actor is absent from
    /// `cutoff` entirely (that actor was unseen when the snapshot was
    /// taken, so all of its ops are still pending). This is the per-actor
    /// generalization of [`Self::ops_since`] and the delta the boot path
    /// replays on top of a [`crate::snapshot::SnapshotBody`].
    ///
    /// Default impl filters [`Self::all_ops`]; backends may override for
    /// efficiency. Correctness — never dropping a low-HLC op from a
    /// lagging peer — lives here, not in the override.
    fn ops_since_per_actor(
        &self,
        cutoff: &BTreeMap<ActorId, Hlc>,
    ) -> Result<Vec<LogOp>, StorageError> {
        Ok(self
            .all_ops()?
            .into_iter()
            .filter(|op| match cutoff.get(&op.actor) {
                Some(c) => op.ts > *c,
                None => true,
            })
            .collect())
    }

    /// Shrink (or grow) the in-memory op cache to hold at most `cap`
    /// ops. `cap = 0` means "unbounded" — keep every op resident (the
    /// legacy default). Default no-op; [`JsonlStorage`] implements it
    /// for real.
    ///
    /// Called by `Workspace` after boot completes (see
    /// `Workspace::apply_lru_cap`). Boot needs every op in RAM to
    /// re-materialise Yrs `Doc`s via `ops_for_node`; once that's done,
    /// the long-running client can shed the cold history.
    ///
    /// Implementations must be idempotent and safe to call from any
    /// point in the lifecycle.
    fn resize_cache(&mut self, _cap: usize) {}
}
