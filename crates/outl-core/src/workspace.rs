//! Workspace — the top-level container.
//!
//! A `Workspace` owns the materialized tree, the in-memory op log, the
//! storage backend, and the per-block Yrs `Doc`s that hold block text.
//! CLI and TUI consume the workspace; they don't reach into `Tree` or
//! `OpLog` directly.
//!
//! This module exposes a deliberately minimal API:
//!
//! - [`Workspace::open_in_memory`] / [`Workspace::open_with_storage`]
//! - [`Workspace::apply`] — accept an op, route to tree + storage + Yrs
//! - read-only accessors for the tree, log, and block text
//!
//! Higher-level methods (page CRUD, journal CRUD, block edit shortcuts)
//! live in `outl-actions` and `outl-cli`, not here.

use crate::content::ContentStore;
use crate::id::{ActorId, NodeId};
use crate::log::OpLog;
use crate::op::{op_node, LogOp, Op};
use crate::storage::{Storage, StorageError};
use crate::tree::Tree;
use router::{Route, StorageRouter};
use snapshot_policy::SnapshotPolicy;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

mod batch;
mod boot;
mod clock;
mod router;
mod snapshot_api;
mod snapshot_policy;
mod stats;
mod text_history;

pub use text_history::TextRevision;

pub use batch::WorkspaceBatch;

/// Errors a workspace may surface to its caller.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Underlying storage failure.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The snapshot boot cache could not be built or written. Never
    /// fatal to the workspace — the op log is the source of truth, so a
    /// caller logs this and carries on without a cache.
    #[error(transparent)]
    Snapshot(#[from] crate::snapshot::SnapshotError),
}

/// Top-level workspace.
///
/// Construct via [`Workspace::open_in_memory`] for tests or
/// [`Workspace::open_with_storage`] for real backends.
pub struct Workspace {
    /// Filesystem root for the workspace, if any.
    pub root: Option<PathBuf>,
    /// This device's actor id.
    pub actor: ActorId,
    /// Materialized structural tree.
    tree: Tree,
    /// In-memory op log mirroring the storage backend.
    log: OpLog,
    /// Block text content (Yrs docs).
    content: ContentStore,
    /// Which storage owns an op, and how the shards read back as one
    /// log. Holds the global backend plus any per-page shards.
    router: StorageRouter,
    /// When to write the local boot cache, and the workers doing it.
    /// Policy, not document state — see [`SnapshotPolicy`].
    snapshots: SnapshotPolicy,
    /// Whether `self.log` carries every op ever persisted (`true` after
    /// full replay) or only the delta posted after a snapshot cutoff
    /// (`false` after snapshot boot). When `false`, `Doc` rebuilds that
    /// need the full `Edit` history for a node load it from storage via
    /// [`Self::ensure_doc_for_edit`] — the in-memory log alone would
    /// miss pre-snapshot edits and produce a wrong Doc state (#129).
    log_complete: bool,
    /// Deferred-persistence depth. `0` means every [`Self::apply`]
    /// persists immediately (the default). `> 0` means a
    /// [`WorkspaceBatch`] guard is active: `apply` still runs the full
    /// dedup + Yrs + CRDT path but buffers the persist into `pending`,
    /// and the outermost guard flushes it with one `append_ops` per
    /// destination on commit/drop. See [`Self::begin_batch`].
    batch_depth: u32,
    /// Buffered persists awaiting the outermost batch flush. Each entry
    /// pairs an op with the storage route resolved **at apply time**
    /// (page routing walks the tree the op may itself have mutated). The
    /// ops already live in the CRDT + in-memory log; this buffer is only
    /// the not-yet-durable persistence queue.
    pending: Vec<(Route, LogOp)>,
}

impl Workspace {
    /// Open an in-memory workspace. Useful for tests.
    pub fn open_in_memory(actor: ActorId) -> Result<Self, WorkspaceError> {
        let storage = crate::storage::MemoryStorage::new();
        Self::open_with_storage(actor, Box::new(storage), None)
    }

    /// Open a workspace backed by a given storage implementation.
    ///
    /// Tries to boot from a snapshot first (O(current state) + the ops
    /// posted since the snapshot); on any failure — missing snapshot,
    /// hash mismatch, future schema — falls back to a full replay of
    /// the op log. Snapshot is purely a boot cache; the op log stays
    /// the single source of truth.
    ///
    /// In both paths, block text is materialized so the workspace is
    /// ready to read from the moment this returns.
    pub fn open_with_storage(
        actor: ActorId,
        storage: Box<dyn Storage>,
        root: Option<PathBuf>,
    ) -> Result<Self, WorkspaceError> {
        let snapshots_dir = root.as_ref().map(|r| r.join(".outl").join("snapshots"));

        let mut ws = Self {
            root,
            actor,
            tree: Tree::new(),
            log: OpLog::new(),
            content: ContentStore::default(),
            router: StorageRouter::new(storage),
            snapshots: SnapshotPolicy::new(snapshots_dir),
            log_complete: true,
            batch_depth: 0,
            pending: Vec::new(),
        };

        let booted_from_snapshot = match ws.boot_from_snapshot() {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                warn!("snapshot boot failed, falling back to full replay: {e}");
                false
            }
        };

        if booted_from_snapshot {
            // The in-memory log only has delta ops; `Doc` rebuilds that
            // need the full `Edit` history load it from storage (#129).
            ws.log_complete = false;
        } else {
            ws.boot_from_full_replay()?;
        }

        Ok(ws)
    }

    /// Ensure the Yrs `Doc` for `node` is in the content cache with the
    /// FULL `Edit` history applied.
    ///
    /// After snapshot boot, `self.log` only carries delta ops (the
    /// pre-snapshot ops live in storage). Rebuilding a `Doc` from the
    /// incomplete log would miss those edits, producing a wrong state
    /// vector and, in turn, updates that concatenate instead of replace
    /// on full replay (#129). When `log_complete` is `false`, we load
    /// the node's entire `Edit` history from storage before the
    /// `ContentStore`'s internal `ensure_doc` kicks in.
    fn ensure_doc_for_edit(&mut self, node: NodeId) {
        if self.content.is_cached(node) {
            return;
        }
        if self.log_complete {
            return; // ContentStore::ensure_doc rebuilds from the full log.
        }
        let mut ops = match self.ops_for_node_combined(node) {
            Ok(o) => o,
            Err(e) => {
                warn!("could not load ops for {node} from storage: {e}");
                return;
            }
        };
        // Sort by HLC before replay: `ops_for_node`'s return order is
        // backend-dependent, and rebuilding the Doc in the same order as the
        // full-replay path keeps the state vector deterministic across
        // backends and sessions.
        ops.sort_by_key(|l| l.ts);
        let updates: Vec<&[u8]> = ops
            .iter()
            .filter_map(|l| match &l.op {
                Op::Edit {
                    node: n, text_op, ..
                } if *n == node => Some(text_op.as_slice()),
                _ => None,
            })
            .collect();
        self.content.cache_doc(node, updates.into_iter());
    }

    /// Whether applying `op` would leave the materialized tree exactly
    /// as it is — a redundant `Create` / `Move` / `SetProp` for a node
    /// that already has that parent+position / property value.
    ///
    /// The `.md`-reconcile diff defensively re-emits `Create` + `Move`
    /// (+ a `SetProp` per property) for **every** block on **every**
    /// commit. Each is idempotent on the tree but still costs one
    /// fsynced append, so a one-block edit on an 11-block page turned
    /// into 23 persisted ops (≈2 per block) — the slow commit, and an op
    /// log that grew by the whole page on every keystroke (the slow
    /// boot). Callers filter these out so only ops that actually change
    /// something get logged. `Edit` is never a no-op here (its Yrs delta
    /// is already empty when text is unchanged); a `Move` whose target
    /// differs is never a no-op, so cycle-forming moves are still logged
    /// (invariant #5).
    pub fn op_is_noop(&self, op: &Op) -> bool {
        match op {
            Op::Create {
                node,
                parent,
                position,
            } => {
                self.tree.contains(*node)
                    && self.tree.parent(*node) == Some(*parent)
                    && self.tree.position(*node) == Some(position)
            }
            Op::Move {
                node,
                new_parent,
                position,
                ..
            } => {
                self.tree.contains(*node)
                    && self.tree.parent(*node) == Some(*new_parent)
                    && self.tree.position(*node) == Some(position)
            }
            Op::SetProp {
                node, key, value, ..
            } => self.tree.property(*node, key) == value.as_ref(),
            _ => false,
        }
    }

    /// Apply an op locally and persist it.
    ///
    /// The op is appended to the in-memory log, dispatched to the Yrs
    /// content store if it's an `Edit`, and persisted to storage. If
    /// storage fails, the in-memory state is still mutated — the caller
    /// is responsible for surfacing the error and/or invoking `outl doctor`.
    pub fn apply(&mut self, op: LogOp) -> Result<(), WorkspaceError> {
        let is_new = !self.log.contains_ts(&op.ts);
        if let Op::Edit { node, text_op } = &op.op {
            // Merge the update into the block's text. The Doc is rebuilt
            // from the log here, before this op is appended below, so the
            // merge sees the prior state.
            self.ensure_doc_for_edit(*node);
            self.content.merge_update(*node, &self.log, text_op);
        }
        let ts = op.ts;
        self.tree.apply_op(&mut self.log, op);

        // Only persist ops the tree didn't already have. `apply_op`
        // deduplicates via `contains_ts`, but the storage append below
        // is unconditional without this guard — a re-delivered op
        // (sync replay, plugin pull) would write a duplicate line.
        if !is_new {
            return Ok(());
        }

        // Persist the op **the tree recorded**, not the one the caller
        // handed in. `do_op` fills `old_parent` / `old_position` /
        // `old_value` on its way through, and those are the caller's
        // guesses until it does: `block::moves::move_to` reads them off
        // the tree and is right, while the reconcile and import paths
        // pass `root` / `None` and are not. Writing the caller's copy
        // left the file disagreeing with the resident log for 65,141 of
        // 65,703 `Move` ops and every one of 14,191 `SetProp` ops in the
        // reference workspace.
        //
        // Convergence never depended on this, which is why it went
        // unnoticed: every path into the **resident log** runs `do_op`,
        // which overwrites the fields before `undo_op` (its only reader,
        // and only from `apply_op`) can see them. Note the narrower
        // claim: peer-sync ingest writes a line straight into
        // `ops-<actor>.jsonl` without passing through here, so a peer's
        // stored op carries *their* derivation until this device's next
        // boot replay runs `do_op` over it. What did depend on it is anything
        // reading the log **as data** — `outl_actions::timeline` has to
        // fold the parent trail from `Create.parent` / `Move.new_parent`
        // precisely because this field lied, and historical ops still
        // carry the old values, so that defence stays.
        let op = self
            .log
            .get_by_ts(&ts)
            .cloned()
            .expect("apply_op appended the op it was handed");

        // Deferred-persistence mode: a `WorkspaceBatch` guard is live, so
        // buffer the op instead of fsyncing it now. The route is resolved
        // here (not at flush time) because page routing walks the tree the
        // op may have just mutated. The snapshot trigger is skipped too —
        // the outermost guard fires it once for the whole batch. See
        // `workspace::batch`.
        if self.batch_depth > 0 {
            // The buffer must stay single-actor: `JsonlStorage::append_ops`
            // rejects the whole batch if any op is foreign, so a stray
            // foreign op would drop the entire flush. See `begin_batch`.
            debug_assert_eq!(
                op.actor, self.actor,
                "batch buffer only carries ops from the local actor"
            );
            let route = self.route_for_op(&op);
            self.pending.push((route, op));
            return Ok(());
        }

        // Route to the right storage. If the op's node belongs to a
        // registered page, write to that page's shard; otherwise write
        // to the global storage (legacy behaviour).
        let route = self.router.route(&self.tree, op_node(&op.op));
        self.router.append_op(&route, &op)?;

        // Background snapshot trigger. `snapshot_threshold = 0` is the
        // opt-out (CLI). When the threshold is crossed we build the
        // `SnapshotBody` inline (cheap-ish: 3 HashMap clones + text map
        // clone + one sha256; the *write* is what's expensive) and hand
        // it off to a worker thread so the user never blocks on fsync.
        // Failure inside the worker is logged and discarded — snapshot
        // is a cache, not source of truth.
        self.trigger_snapshot(1);
        Ok(())
    }

    /// Every op that names `node`, oldest first, read from **storage**.
    ///
    /// Reads **storage**, never the resident log: the resident log is
    /// boot-mode dependent (a snapshot boot leaves it holding only the
    /// post-cutoff delta), so anything asking about a node's *past* has
    /// to go to the log on disk or risk a silently short answer.
    ///
    /// This is what makes the op log verifiable as data, which is the
    /// point of `apply` persisting what `do_op` recorded. The test that
    /// pins that reads through here.
    ///
    /// # Cost
    ///
    /// One index-driven read set per node — O(ops-of-node), not O(log),
    /// but it goes to **storage** on every call: with `JsonlStorage` that
    /// is a seek plus a line parse per op, and nothing caches in front of
    /// it. Fine once per user action (a history panel, a recovery scan);
    /// not something to call per block in a render loop.
    pub fn ops_for_node(&self, node: NodeId) -> Result<Vec<LogOp>, WorkspaceError> {
        Ok(self.ops_for_node_combined(node)?)
    }

    /// Read-only access to the materialized tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Read-only access to the in-memory op log.
    ///
    /// After a snapshot boot this is only the post-cutoff delta, not the whole
    /// op log. Use [`Self::total_ops`] when you need the persisted total.
    pub fn log(&self) -> &OpLog {
        &self.log
    }

    /// Block text content, if the block has any.
    ///
    /// After a full-replay boot the string may not be materialized yet
    /// (that O(all blocks) pass is deferred, #179); this rebuilds it from
    /// the in-memory log on first read and caches it. The log is complete
    /// on the full-replay path (`log_complete == true`), and snapshot boot
    /// leaves nothing deferred, so this never needs storage.
    pub fn block_text(&self, node: NodeId) -> Option<String> {
        // Ahead of the cache, because for these the cache is wrong: the
        // snapshot body carries this node's text as of its cutoff and the
        // delta replayed on top holds a later `Edit`. The full history is
        // in storage, never in the resident log, which after a snapshot
        // boot holds only the delta — rebuilding from that would drop
        // every pre-cutoff edit (#129).
        if self.content.is_stale(node) {
            match self.ops_for_node_combined(node) {
                Ok(mut ops) => {
                    // `ops_for_node`'s order is backend-dependent
                    // (`JsonlStorage`'s in-memory cache can be unsorted
                    // after appends). Sort by HLC so the `Doc` is rebuilt
                    // in the same order the full-replay path uses.
                    ops.sort_by_key(|l| l.ts);
                    let updates: Vec<&[u8]> = ops
                        .iter()
                        .filter_map(|l| match &l.op {
                            Op::Edit {
                                node: n, text_op, ..
                            } if *n == node => Some(text_op.as_slice()),
                            _ => None,
                        })
                        .collect();
                    if !updates.is_empty() {
                        return Some(self.content.text_from_edits(node, updates.into_iter()));
                    }
                }
                // The eager pass logged and moved on, leaving the
                // snapshot's string in place; so does this. A read that
                // cannot reach storage returns the best text there is
                // rather than nothing, and the node stays marked so the
                // next read tries again.
                Err(e) => warn!("deferred re-materialize for {node}: {e}"),
            }
        }
        // Cache hit (snapshot body, or a prior read) — cheap.
        if let Some(s) = self.content.cached(node) {
            return Some(s);
        }
        // A node with no recorded `Edit` history reads back as `None`.
        if !self.content.is_pending(node) {
            return None;
        }
        // Resolve the node's `Edit` updates through the log's IN-MEMORY
        // per-node index — O(edits-of-node), no log scan, no disk. The old
        // `content.text(node, &self.log)` scanned the whole log per block
        // (O(log) each — pathological after a full replay of a 200k-op vault:
        // the "journal takes forever to render" regression); the on-disk
        // `ops_for_node` path did a cold seek per op (worse). This branch only
        // runs post-full-replay — a snapshot boot leaves `pending` empty
        // (delta nodes are re-materialized at boot). Same edits, same HLC
        // order, same text.
        Some(
            self.content
                .text_from_edits(node, self.log.edit_updates(node)),
        )
    }

    /// Number of live Yrs `Doc`s currently resident in the content cache.
    ///
    /// Test-only window into the bound that keeps large vaults under the
    /// iOS memory limit (issue #108).
    #[cfg(test)]
    fn live_doc_count(&self) -> usize {
        self.content.live_doc_count()
    }

    /// Number of block strings currently materialized in the content
    /// cache.
    ///
    /// A window into the lazy full-replay boot path (#179): after a
    /// full-replay boot this is ~0 and grows only as blocks are read via
    /// [`Self::block_text`]. Downstream crates assert on it to prove a
    /// read path (e.g. the backlinks index build) does **not** force the
    /// whole workspace to materialize — the regression that froze the
    /// app on open. Cheap (a map length); safe to call in production.
    pub fn resident_text_count(&self) -> usize {
        self.content.resident_text_count()
    }

    /// Build a Yrs `update_v1` payload that, once wrapped in
    /// `Op::Edit { node, text_op }` and pushed through [`Self::apply`],
    /// rewrites the block's text to `new_text` exactly.
    ///
    /// **Side effect:** the workspace's own content `Doc` for `node`
    /// is mutated in-place to match `new_text`. The returned update
    /// encodes exactly that mutation, captured via the Doc's state
    /// vector. Yrs is idempotent on `apply_update`, so the subsequent
    /// `Workspace::apply(LogOp::Edit { … })` is a no-op on the local
    /// Doc but still appends to the log and propagates to peers.
    ///
    /// Returns an empty `Vec` when the requested change is a no-op.
    pub fn build_text_replace_update(&mut self, node: NodeId, new_text: &str) -> Vec<u8> {
        let current = self.block_text(node).unwrap_or_default();
        if current == new_text {
            return Vec::new();
        }
        self.ensure_doc_for_edit(node);
        self.content.replace_text(node, &self.log, new_text)
    }

    /// Re-boot the workspace from all registered storages (Global +
    /// PerPage). Called by clients after registering per-page shards
    /// via [`Self::register_page_storage`] — the initial boot only
    /// sees the Global storage, so a re-boot is needed to load the
    /// per-page ops into the materialized tree. RFC #137 Phase B.
    pub fn reboot_with_all_storages(&mut self) -> Result<(), WorkspaceError> {
        self.tree = Tree::new();
        self.log = OpLog::new();
        self.content = ContentStore::default();
        self.log_complete = true;
        self.boot_from_full_replay()?;
        Ok(())
    }

    /// Whether any per-page storage shards have been registered.
    pub fn has_page_storages(&self) -> bool {
        self.router.has_pages()
    }

    /// Register a per-page storage backend. The client (CLI / TUI /
    /// desktop / mobile) calls this after opening the workspace if the
    /// workspace uses the per-page shard layout (RFC #137 Phase B).
    /// Ops whose node belongs to `slug` will be routed to this storage
    /// instead of the global one.
    pub fn register_page_storage(&mut self, slug: &str, storage: Box<dyn Storage>) {
        self.router.register_page(slug, storage);
    }

    /// Register a page root → slug mapping. The client calls this for
    /// every page/journal it knows about (read from sidecars via
    /// `outl-md`). `apply` uses this to resolve which page an op
    /// belongs to by walking the parent chain up to a page root.
    pub fn register_page_root(&mut self, root_id: NodeId, slug: &str) {
        self.router.register_root(root_id, slug);
    }

    /// Walk `tree.parent(node)` up until we hit a registered page root.
    /// Returns the slug of that page, or `None` if the chain dead-ends
    /// at the workspace root without finding one.
    ///
    /// Public so a client can name the page a synced block belongs to (the
    /// pairing-screen progress feed resolves freshly-received block ids to
    /// their page/journal slug). Requires the page roots to be registered
    /// (`register_page_root`) — an unregistered or not-yet-materialized node
    /// returns `None`, so callers treat it as best-effort.
    pub fn slug_for_node(&self, node: NodeId) -> Option<String> {
        self.router.slug_for_node(&self.tree, node)
    }

    /// Merge ops from the global storage and every registered
    /// per-page storage, sorted by HLC. Used by boot and read-side
    /// accessors (`all_ops`, `ops_since`, `ops_for_node`, etc.).
    fn all_ops_combined(&self) -> Result<Vec<LogOp>, StorageError> {
        self.router.all_ops()
    }

    /// Merge the per-actor delta (see [`Storage::ops_since_per_actor`])
    /// from every storage, sorted by HLC. The snapshot boot path replays
    /// this on top of the snapshot body.
    fn ops_since_per_actor_combined(
        &self,
        cutoff: &std::collections::BTreeMap<ActorId, crate::hlc::Hlc>,
    ) -> Result<Vec<LogOp>, StorageError> {
        self.router.ops_since_per_actor(cutoff)
    }

    /// Merge ops for `node` from every storage.
    fn ops_for_node_combined(&self, node: NodeId) -> Result<Vec<LogOp>, StorageError> {
        self.router.ops_for_node(node)
    }

    /// Merge `last_ts_per_actor` from every storage — the per-actor
    /// high-water mark used as the snapshot cutoff.
    fn last_ts_per_actor_combined(
        &self,
    ) -> Result<HashMap<ActorId, crate::hlc::Hlc>, StorageError> {
        self.router.last_ts_per_actor()
    }

    /// Apply the LRU cap configured by the client, after boot has
    /// finished re-materialising Yrs `Doc`s.
    ///
    /// Boot needs every op in RAM so `ops_for_node` (used to rebuild
    /// `Doc`s for nodes edited after the snapshot cutoff) sees the full
    /// `Edit` history. Once boot is done, the long-running client can
    /// shed cold history: `cap = 50_000` keeps the most-recent ops
    /// resident; older ones come back from disk via the offset index.
    /// RFC #137 Phase A.
    ///
    /// `cap = 0` means "unbounded" — keep every op resident (legacy
    /// default). Idempotent and safe to call at any point in the
    /// lifecycle (clients may resize on config change).
    pub fn apply_lru_cap(&mut self, cap: usize) {
        self.router.resize_cache(cap);
    }
}

#[cfg(test)]
mod tests;
