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
use crate::snapshot::{self, SnapshotBody};
use crate::storage::{Storage, StorageError};
use crate::tree::Tree;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::thread::JoinHandle;
use tracing::{debug, warn};

mod batch;
mod text_history;

pub use text_history::TextRevision;

use batch::BatchRoute;
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
    /// Pluggable storage backend (Global scope — the legacy
    /// single-file-per-actor layout). Per-page shards live in
    /// `page_storages`.
    storage: Box<dyn Storage>,
    /// Per-page storage backends (Phase B of RFC #137). Keyed by page
    /// slug. Empty for workspaces that haven't migrated to per-page
    /// shards. When non-empty, `apply` routes each op to the storage
    /// that owns the op's node.
    page_storages: HashMap<String, Box<dyn Storage>>,
    /// `NodeId → slug` map for page roots. Populated by the client
    /// (which reads sidecars via `outl-md`) via
    /// [`Self::register_page_root`]. `apply` walks the parent chain
    /// from an op's node up to a page root, then uses this map to
    /// find the slug and route to the right `page_storages` entry.
    page_root_to_slug: HashMap<NodeId, String>,
    /// `<root>/.outl/snapshots` when `root` is set, `None` for
    /// in-memory workspaces. Background snapshot writes go straight
    /// here — they don't go through `storage` (the snapshot is a local
    /// cache, not part of the source-of-truth op log).
    snapshots_dir: Option<PathBuf>,
    /// Background snapshot writers still in flight. `apply` drains
    /// finished handles on every trigger (cheap, non-blocking) so the
    /// list stays bounded; `wait_for_snapshots` joins the rest.
    snapshot_workers: Vec<JoinHandle<()>>,
    /// Number of ops applied since the last successful snapshot write.
    /// `apply` increments this and spawns a snapshot worker once it
    /// crosses `snapshot_threshold`. Reset to `0` on every spawn and
    /// on policy change.
    ops_since_snapshot: u32,
    /// Trigger threshold for background snapshot writes inside `apply`.
    /// `0` disables the in-band trigger (the CLI sets this — it's
    /// ephemeral and shouldn't churn the snapshots dir).
    snapshot_threshold: u32,
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
    pending: Vec<(BatchRoute, LogOp)>,
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
            storage,
            page_storages: HashMap::new(),
            page_root_to_slug: HashMap::new(),
            snapshots_dir,
            snapshot_workers: Vec::new(),
            ops_since_snapshot: 0,
            snapshot_threshold: Workspace::DEFAULT_SNAPSHOT_THRESHOLD,
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

    /// Whether this workspace booted by adopting a snapshot (so the
    /// in-memory log holds only the post-cutoff delta) rather than a full
    /// replay (log holds every op).
    ///
    /// A receive-only device whose snapshot was rejected/absent/stale
    /// full-replays, leaving a huge resident log that makes `block_text`
    /// materialization O(log) per block (it scans the log for each node's
    /// `Edit`s). The caller uses this to persist a FRESH snapshot right
    /// after such a boot, so the next boot adopts it (delta-only log) and
    /// materialization stays cheap.
    pub fn booted_from_snapshot(&self) -> bool {
        !self.log_complete
    }

    /// Default `apply`-count between in-band snapshot writes. Clients
    /// override with [`Self::set_snapshot_policy`] from `[snapshot]`
    /// in `outl.toml`. The CLI forces `0` to opt out.
    const DEFAULT_SNAPSHOT_THRESHOLD: u32 = 10_000;

    /// Try to hydrate the workspace from a snapshot + the ops posted
    /// since its cutoff. Returns `Ok(false)` when there's nothing to
    /// load (so the caller falls through to [`Self::boot_from_full_replay`]).
    fn boot_from_snapshot(&mut self) -> Result<bool, WorkspaceError> {
        // Snapshots are a local boot cache owned by the workspace, read
        // straight from `<root>/.outl/snapshots` — not routed through the
        // storage backend (the op log). An in-memory workspace
        // (`root = None`) has nowhere to read from, so it never boots
        // from snapshot.
        let snapshots_dir = match &self.snapshots_dir {
            Some(d) => d.clone(),
            None => return Ok(false),
        };
        // Prefer this device's own snapshot; when it has none yet (a fresh
        // paired device), adopt a peer's snapshot so we skip a full replay
        // of a 200k-op log. Local input is preserved either way — the
        // per-actor delta below replays every op above the snapshot's
        // cutoff, including this device's own. See `read_best_from_disk`.
        let body = match snapshot::read_best_from_disk(&snapshots_dir, self.actor) {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(false),
            Err(e) => {
                // Corrupt / stale / unreadable snapshot is never fatal —
                // it's a cache. Fall back to a full op-log replay.
                warn!("snapshot unusable, falling back to full replay: {e}");
                return Ok(false);
            }
        };

        // Compute the delta the snapshot hasn't seen — per actor, so a
        // low-HLC op from an actor the snapshot never saw isn't dropped
        // below another actor's high-water mark (#156).
        let delta = match self.ops_since_per_actor_combined(&body.cutoff) {
            Ok(d) => d,
            Err(e) => {
                // An op the index lists but the file won't return. NOT
                // fatal, and emphatically not something to skip past:
                // the cutoff of the snapshot we write next comes from
                // the index, so adopting a delta that quietly omitted
                // this op would record it as "already folded in" and no
                // future boot would replay it. Fall back to a full
                // sequential replay, which re-reads the file and can
                // still recover everything around the damaged line.
                warn!("snapshot delta unreadable, falling back to full replay: {e}");
                return Ok(false);
            }
        };

        // CONVERGENCE GUARD (invariant #1). The snapshot body is an opaque
        // MATERIALIZED tree, not a reorderable log, so applying the delta on
        // top equals a full replay ONLY when the delta is a pure temporal
        // SUFFIX — every delta op newer than every op folded into the body.
        // If a delta op sorts at/below a body op, the CRDT would need to
        // reorder it against ops that live only in the (opaque) tree, so a
        // cycle-forming `Move` can resolve the opposite way and the tree
        // diverges from a full replay. The per-actor cutoff makes this
        // reachable — a peer op below our cutoff, or an adopted peer
        // snapshot that excludes all our local ops. Bail to a full replay:
        // correct over fast. (No-silent-loss #5 holds regardless — the op is
        // always in the log; this only decides the materialization order.)
        if let Some(max_body_hlc) = body.cutoff.values().max().copied() {
            if delta.iter().any(|op| op.ts <= max_body_hlc) {
                return Ok(false);
            }
        }

        // Safe to adopt: hydrate the materialized tree + block text from the
        // snapshot body, then apply the suffix delta on top. No `Edit` ops
        // are replayed for the body nodes; `block_text` already carries
        // their settled string. Convert BTreeMap → HashMap to match the
        // runtime representation (ContentStore's hot path is a HashMap).
        self.tree = Tree::from_parts(
            body.nodes.into_iter().collect(),
            body.properties.into_iter().collect(),
            body.collapsed.into_iter().collect(),
            body.snoozed.into_iter().collect(),
        );
        self.content = ContentStore::from_text_map(body.block_text.into_iter().collect());

        // `Edit` ops in the delta are re-materialized in the loop below;
        // the rest of the snapshot's text is still authoritative.
        let mut edited_in_delta: HashSet<NodeId> = HashSet::new();
        for op in delta {
            if let Op::Edit { node, .. } = &op.op {
                edited_in_delta.insert(*node);
            }
            self.tree.apply_op(&mut self.log, op);
        }

        // Re-materialize only the nodes whose text changed after the
        // snapshot. The in-memory log only has delta ops (pre-snapshot
        // ops are in storage), so we load the FULL `Edit` history for
        // each edited node from storage. Replaying everything through a
        // fresh `Doc` — pre-snapshot edits included — is what makes the
        // resulting text match the full-replay path (#129).
        let rematerialized_count = edited_in_delta.len();
        for node in &edited_in_delta {
            let mut ops = match self.ops_for_node_combined(*node) {
                Ok(o) => o,
                Err(e) => {
                    warn!("skipping re-materialize for {node}: {e}");
                    continue;
                }
            };
            // `ops_for_node`'s order is backend-dependent (JsonlStorage's
            // in-memory cache can be unsorted after appends). Sort by HLC so
            // the Doc is rebuilt in the same order as the full-replay path.
            ops.sort_by_key(|l| l.ts);
            let updates: Vec<&[u8]> = ops
                .iter()
                .filter_map(|l| match &l.op {
                    Op::Edit {
                        node: n, text_op, ..
                    } if n == node => Some(text_op.as_slice()),
                    _ => None,
                })
                .collect();
            if !updates.is_empty() {
                self.content.materialize(*node, updates.into_iter());
            }
        }

        debug!(
            "booted from snapshot ({} actors in cutoff, {} nodes, {} delta ops, \
             {rematerialized_count} nodes re-materialized)",
            body.cutoff.len(),
            self.tree.node_count(),
            self.log.len(),
        );

        Ok(true)
    }

    /// Boot by replaying every op in storage. The historical path; used
    /// as the snapshot-miss fallback and by `open_in_memory`.
    fn boot_from_full_replay(&mut self) -> Result<(), WorkspaceError> {
        // Pass 1: structural. Apply every op to the tree (`Edit` is a
        // no-op there) and the log. This keeps the tree FULLY materialized;
        // only block text goes lazy below.
        let ops = self.all_ops_combined()?;
        for op in ops {
            self.tree.apply_op(&mut self.log, op);
        }

        // Pass 2: defer text. Materializing every block's `Doc` here was
        // O(all blocks) — a major boot freeze on large snapshotless vaults
        // (a 66k-block install-clean froze the mobile app, #179). Instead
        // record which nodes carry `Edit` history (indices-free, one cheap
        // scan) and let `block_text` rebuild each string lazily on first
        // read. `self.log` is complete here (`log_complete == true`), so a
        // lazy rebuild sees the node's full `Edit` set and produces
        // byte-identical text to the old eager pass.
        let edited: HashSet<NodeId> = self
            .log
            .iter()
            .filter_map(|logged| match &logged.op {
                Op::Edit { node, .. } => Some(*node),
                _ => None,
            })
            .collect();
        self.content.set_pending(edited);

        // Seed the snapshot counter so a long-lived workspace opened
        // without a snapshot on disk doesn't have to wait a full
        // `threshold` worth of new ops before producing one. If the log
        // already crosses the threshold, the next `apply` snapshots.
        self.ops_since_snapshot = (self.log.len() as u32).min(self.snapshot_threshold);

        Ok(())
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
        let slug = op_node(&op.op).and_then(|node| self.slug_for_node(node));
        match slug {
            Some(ref slug) if self.page_storages.contains_key(slug) => {
                if let Some(s) = self.page_storages.get_mut(slug) {
                    s.append_op(&op)?;
                }
            }
            _ => {
                self.storage.append_op(&op)?;
            }
        }

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

    /// Assemble a [`SnapshotBody`] from the current materialized state,
    /// or `None` when nothing is persisted yet. Single builder for both
    /// the background [`Self::spawn_background_snapshot`] and the
    /// synchronous [`Self::save_snapshot`] paths so the cutoff and the
    /// serialized shape can never drift between them.
    ///
    /// The cutoff is the per-actor high-water mark across **all** storage
    /// (global + per-page shards), so the snapshot records exactly which
    /// of each actor's ops its materialized state already folds in.
    /// Materialize every still-deferred block's text via the per-node index
    /// (O(edits) each), so a follow-up `materialized_text` finds it cached
    /// instead of falling back to its O(log)-per-block log scan. Without this,
    /// snapshotting a full-replay-booted 200k-op vault is O(blocks × log).
    fn force_materialize_pending(&self) {
        for node in self.content.pending_snapshot() {
            // Index-driven (see `block_text`); caches the string + clears
            // `pending`. A node that fails to resolve is left pending — the
            // snapshot then omits it and the next boot rebuilds it lazily.
            let _ = self.block_text(node);
        }
    }

    fn build_snapshot_body(&self) -> Result<Option<SnapshotBody>, WorkspaceError> {
        let cutoff = self.last_ts_per_actor_combined()?;
        if cutoff.is_empty() {
            return Ok(None);
        }
        // Resolve deferred text through the index first (cheap), so the
        // `materialized_text` call below is a pure cache read.
        self.force_materialize_pending();
        let (nodes, properties, collapsed, snoozed) = self.tree.snapshot_parts();
        // Convert to BTreeMap/BTreeSet for canonical (order-stable)
        // serialization — see `snapshot::SnapshotBody` for why.
        // An encode failure surfaces as `WorkspaceError::Snapshot`; both
        // callers warn and skip the cache rather than lose the workspace.
        Ok(Some(SnapshotBody::from_parts(
            self.actor,
            cutoff.into_iter().collect(),
            nodes.iter().map(|(k, v)| (*k, v.clone())).collect(),
            properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            collapsed.iter().copied().collect(),
            snoozed.iter().map(|(k, v)| (*k, *v)).collect(),
            // Force-materialize any block whose text the lazy read path
            // deferred at boot (#179) so the snapshot carries every
            // block's string — an incomplete map would silently drop text
            // on the next snapshot boot.
            self.content
                .materialized_text(&self.log)
                .into_iter()
                .collect(),
        )?))
    }

    /// Build a `SnapshotBody` from the current materialized state and
    /// hand it to a worker thread that runs [`snapshot::write_to_disk`].
    /// Non-blocking: the encode + fsync + rename happen off the calling
    /// thread. If the body can't be built (e.g. empty log) we no-op.
    fn spawn_background_snapshot(&mut self) {
        let snapshots_dir = match &self.snapshots_dir {
            Some(p) => p.clone(),
            None => return,
        };
        let body = match self.build_snapshot_body() {
            Ok(Some(b)) => b,
            Ok(None) => return,
            Err(e) => {
                warn!("skipping background snapshot (cutoff read failed): {e}");
                return;
            }
        };

        let handle = std::thread::Builder::new()
            .name(format!("outl-snapshot-{}", self.actor))
            .spawn(move || {
                if let Err(e) = snapshot::write_to_disk(&snapshots_dir, &body) {
                    warn!("background snapshot write failed (non-fatal): {e}");
                }
            })
            .expect("spawn snapshot worker");
        self.snapshot_workers.push(handle);
    }

    /// Block until every background snapshot worker finishes.
    ///
    /// Call on graceful shutdown so a long-lived client doesn't exit
    /// with a snapshot write still in flight (which would race the
    /// process exit and could leave a stale `.tmp` behind). Errors
    /// inside the workers are already logged; this just joins.
    pub fn wait_for_snapshots(&mut self) {
        let workers = std::mem::take(&mut self.snapshot_workers);
        for h in workers {
            if let Err(e) = h.join() {
                warn!("snapshot worker panicked: {e:?}");
            }
        }
    }

    /// Configure when the workspace writes snapshots to disk during
    /// `apply`. Pass `enabled = false` to opt out (the CLI does this
    /// — it's ephemeral and shouldn't churn the snapshots dir). The
    /// threshold is the number of `apply` calls between snapshot
    /// writes; values smaller than 1 are clamped to 1 so we don't
    /// snapshot on every single op.
    ///
    /// Reads `[snapshot]` from `outl.toml` via `outl-config` — clients
    /// wire it up after loading config:
    ///
    /// ```ignore
    /// ws.set_snapshot_policy(cfg.snapshot.enabled, cfg.snapshot.op_threshold);
    /// ```
    pub fn set_snapshot_policy(&mut self, enabled: bool, threshold: u32) {
        self.snapshot_threshold = if enabled { threshold.max(1) } else { 0 };
        self.ops_since_snapshot = 0;
    }

    /// Persist a snapshot of the materialized state under the current
    /// actor, stamped with the latest applied HLC as its cutoff.
    ///
    /// Synchronous — used by clients on graceful shutdown (when they're
    /// about to drop the workspace anyway and don't care that the write
    /// blocks). For the in-band path, [`Self::apply`] spawns a worker
    /// thread that calls the same writer without blocking the caller.
    /// Both paths write straight to `<root>/.outl/snapshots` via
    /// [`snapshot::write_to_disk`]; the snapshot is a local boot cache,
    /// never routed through the storage backend (the op log).
    ///
    /// A no-op (returns `Ok(())`) when the log is empty — there's
    /// nothing worth snapshotting and a zero-cutoff snapshot would
    /// force the next boot to re-replay everything anyway — or when the
    /// workspace has no root (`open_in_memory`), which has nowhere to
    /// write.
    pub fn save_snapshot(&mut self) -> Result<(), WorkspaceError> {
        let snapshots_dir = match &self.snapshots_dir {
            Some(d) => d.clone(),
            None => return Ok(()),
        };
        let body = match self.build_snapshot_body()? {
            Some(b) => b,
            None => return Ok(()),
        };
        snapshot::write_to_disk(&snapshots_dir, &body).map_err(|e| {
            WorkspaceError::Storage(StorageError::Backend(format!("snapshot write: {e}")))
        })?;
        debug!(
            "snapshot saved ({} actors in cutoff, {} nodes)",
            body.cutoff.len(),
            body.nodes.len(),
        );
        Ok(())
    }

    /// Read-only access to the materialized tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Read-only access to the in-memory op log.
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
        !self.page_storages.is_empty()
    }

    /// Register a per-page storage backend. The client (CLI / TUI /
    /// desktop / mobile) calls this after opening the workspace if the
    /// workspace uses the per-page shard layout (RFC #137 Phase B).
    /// Ops whose node belongs to `slug` will be routed to this storage
    /// instead of the global one.
    pub fn register_page_storage(&mut self, slug: &str, storage: Box<dyn Storage>) {
        self.page_storages.insert(slug.to_string(), storage);
    }

    /// Register a page root → slug mapping. The client calls this for
    /// every page/journal it knows about (read from sidecars via
    /// `outl-md`). `apply` uses this to resolve which page an op
    /// belongs to by walking the parent chain up to a page root.
    pub fn register_page_root(&mut self, root_id: NodeId, slug: &str) {
        self.page_root_to_slug.insert(root_id, slug.to_string());
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
        let mut current = node;
        loop {
            if let Some(slug) = self.page_root_to_slug.get(&current) {
                return Some(slug.clone());
            }
            current = self.tree.parent(current)?;
        }
    }

    /// Merge ops from the global storage and every registered
    /// per-page storage, sorted by HLC. Used by boot and read-side
    /// accessors (`all_ops`, `ops_since`, `ops_for_node`, etc.).
    fn all_ops_combined(&self) -> Result<Vec<LogOp>, StorageError> {
        let mut all = self.storage.all_ops()?;
        for s in self.page_storages.values() {
            all.extend(s.all_ops()?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// Merge the per-actor delta (see [`Storage::ops_since_per_actor`])
    /// from every storage, sorted by HLC. The snapshot boot path replays
    /// this on top of the snapshot body.
    fn ops_since_per_actor_combined(
        &self,
        cutoff: &std::collections::BTreeMap<ActorId, crate::hlc::Hlc>,
    ) -> Result<Vec<LogOp>, StorageError> {
        let mut all = self.storage.ops_since_per_actor(cutoff)?;
        for s in self.page_storages.values() {
            all.extend(s.ops_since_per_actor(cutoff)?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// Merge ops for `node` from every storage.
    fn ops_for_node_combined(&self, node: NodeId) -> Result<Vec<LogOp>, StorageError> {
        let mut all = self.storage.ops_for_node(node)?;
        for s in self.page_storages.values() {
            all.extend(s.ops_for_node(node)?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// Merge `last_ts_per_actor` from every storage — the per-actor
    /// high-water mark used as the snapshot cutoff.
    fn last_ts_per_actor_combined(
        &self,
    ) -> Result<HashMap<ActorId, crate::hlc::Hlc>, StorageError> {
        let mut map = self.storage.last_ts_per_actor()?;
        for s in self.page_storages.values() {
            for (actor, ts) in s.last_ts_per_actor()? {
                map.entry(actor)
                    .and_modify(|existing| {
                        if ts > *existing {
                            *existing = ts;
                        }
                    })
                    .or_insert(ts);
            }
        }
        Ok(map)
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
        self.storage.resize_cache(cap);
        for s in self.page_storages.values_mut() {
            s.resize_cache(cap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::DOC_CACHE_CAP;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::op::Op;
    use yrs::{Doc, Text, Transact};

    fn make_op(g: &HlcGenerator, op: Op) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    /// The ghost-block policy, core half (issue #213 item 1, RFC 0129).
    ///
    /// **Never write an op for something the user did not do.** The
    /// `.md` reconcile diff re-emits `Create` + `Move` + a `SetProp`
    /// per property for every block on every commit; each is
    /// idempotent on the tree and each still costs an fsynced append.
    /// A one-block edit on an 11-block page logged 23 ops before this
    /// filter existed — the slow commit, and an op log that grew by
    /// the whole page per keystroke.
    ///
    /// The filter had no direct test: it was only exercised through
    /// `outl-md`'s reconcile, where a regression shows up as "commits
    /// got slower", which nobody notices in CI.
    #[test]
    fn a_redundant_op_is_recognised_as_a_noop() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let tmp = tempfile::TempDir::new().unwrap();
        let storage =
            Box::new(crate::storage::JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

        let node = NodeId::new();
        let pos = Fractional::between(None, None);
        let create = Op::Create {
            node,
            parent: NodeId::root(),
            position: pos.clone(),
        };

        assert!(
            !ws.op_is_noop(&create),
            "a Create for a node that does not exist yet is real work",
        );
        ws.apply(make_op(&g, create.clone())).unwrap();
        assert!(
            ws.op_is_noop(&create),
            "re-emitting the same Create must be recognised as a no-op —              this is the filter that keeps a reconcile from logging the              whole page on every keystroke",
        );

        // A property, then the same property again.
        let set = Op::SetProp {
            node,
            key: "remind".into(),
            value: Some(crate::property::PropValue::Text("9am".into())),
            old_value: None,
        };
        assert!(!ws.op_is_noop(&set), "setting a new property is real work");
        ws.apply(make_op(&g, set.clone())).unwrap();
        assert!(
            ws.op_is_noop(&set),
            "re-setting a property to the value it already has is a no-op",
        );

        // Changing the value is NOT a no-op — guards the guard, since a
        // filter that swallows everything would pass every assertion above.
        let changed = Op::SetProp {
            node,
            key: "remind".into(),
            value: Some(crate::property::PropValue::Text("10am".into())),
            old_value: None,
        };
        assert!(
            !ws.op_is_noop(&changed),
            "a property whose value differs must still be logged",
        );
    }

    /// The dangerous direction: over-filtering silently loses user work.
    ///
    /// A `Move` to a *different* parent is never a no-op, including one
    /// that would form a cycle — invariant 4 says the cycle-forming move
    /// is a no-op **on the materialized tree** and still goes into the
    /// log, because removing it breaks the correctness of future
    /// reordering. If `op_is_noop` ever started answering "true" for
    /// those, the filter would drop them before they were ever written.
    #[test]
    fn a_move_that_changes_the_tree_is_never_filtered_out() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let tmp = tempfile::TempDir::new().unwrap();
        let storage =
            Box::new(crate::storage::JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

        let parent = NodeId::new();
        let child = NodeId::new();
        let pos = Fractional::between(None, None);
        for node in [parent, child] {
            ws.apply(make_op(
                &g,
                Op::Create {
                    node,
                    parent: NodeId::root(),
                    position: pos.clone(),
                },
            ))
            .unwrap();
        }

        // child -> parent: a real move.
        let nest = Op::Move {
            node: child,
            new_parent: parent,
            position: pos.clone(),
            old_parent: NodeId::root(),
            old_position: pos.clone(),
        };
        assert!(!ws.op_is_noop(&nest), "a move to a new parent is real work");
        ws.apply(make_op(&g, nest.clone())).unwrap();
        assert!(ws.op_is_noop(&nest), "re-emitting the same move is a no-op");

        // parent -> child would form a cycle. It must NOT be filtered:
        // invariant 4 requires it in the log even though the tree
        // refuses it.
        let cycle = Op::Move {
            node: parent,
            new_parent: child,
            position: pos.clone(),
            old_parent: NodeId::root(),
            old_position: pos.clone(),
        };
        assert!(
            !ws.op_is_noop(&cycle),
            "a cycle-forming move must still be logged (invariant 4) —              filtering it out breaks the correctness of future reordering",
        );
    }

    /// `Op::Edit` is never filtered here, and the reason is worth pinning:
    /// a Yrs delta for unchanged text is *already empty*, so the emptiness
    /// check belongs upstream where the delta is built. If someone adds an
    /// `Edit` arm to `op_is_noop` that inspects the bytes, they are
    /// duplicating a decision that already has an owner.
    #[test]
    fn edit_is_left_to_the_yrs_delta_to_decide() {
        let actor = ActorId::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let storage =
            Box::new(crate::storage::JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap());
        let ws = Workspace::open_with_storage(actor, storage, None).unwrap();

        assert!(
            !ws.op_is_noop(&Op::Edit {
                node: NodeId::new(),
                text_op: Vec::new(),
            }),
            "Edit must not be filtered by op_is_noop",
        );
    }

    #[test]
    fn open_apply_reload_preserves_state() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        // Use a shared directory: open, write, close, reopen.
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let storage1 = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage1, None).unwrap();
        let n = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        drop(ws);

        let storage2 = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
        let ws2 = Workspace::open_with_storage(actor, storage2, None).unwrap();
        assert_eq!(ws2.tree().node_count(), 1);
        assert_eq!(ws2.tree().parent(n), Some(NodeId::root()));
        assert_eq!(ws2.log().len(), 1);
    }

    #[test]
    fn edit_dispatches_to_content_store() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let n = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();

        // Build a Yrs update locally.
        let doc = Doc::new();
        let text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        text.push(&mut txn, "hello outl");
        let update_bytes = txn.encode_update_v1();
        drop(txn);

        ws.apply(make_op(
            &g,
            Op::Edit {
                node: n,
                text_op: update_bytes,
            },
        ))
        .unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some("hello outl"));
    }

    /// Edit one block, then create + edit a fresh one. Reopening rebuilds
    /// the text of both from the log even though neither Doc was kept
    /// resident across the close.
    #[test]
    fn reopen_rebuilds_text_without_resident_docs() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let storage = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

        let mut ids = Vec::new();
        for i in 0..5 {
            let n = NodeId::new();
            ids.push(n);
            ws.apply(make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            ))
            .unwrap();
            let update = ws.build_text_replace_update(n, &format!("block {i}"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: update,
                },
            ))
            .unwrap();
        }
        drop(ws);

        let storage = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
        let ws2 = Workspace::open_with_storage(actor, storage, None).unwrap();
        for (i, n) in ids.iter().enumerate() {
            assert_eq!(
                ws2.block_text(*n).as_deref(),
                Some(format!("block {i}").as_str())
            );
        }
        // Pass 2 materializes strings and drops every Doc, so nothing is
        // resident right after open — the whole point of issue #108.
        assert_eq!(ws2.live_doc_count(), 0);
    }

    /// Full-replay boot must NOT materialize block text up front (#179):
    /// no string is resident until the first `block_text` read, which then
    /// rebuilds that one block lazily and correctly — byte-identical to
    /// what the old eager pass produced.
    #[test]
    fn full_replay_boot_defers_block_text_materialization() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let storage = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

        // Many edited blocks, plus one create-only block (no text).
        let mut edited = Vec::new();
        for i in 0..64 {
            let n = NodeId::new();
            edited.push((n, format!("block {i} ☃")));
            ws.apply(make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            ))
            .unwrap();
            let update = ws.build_text_replace_update(n, &format!("block {i} ☃"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: update,
                },
            ))
            .unwrap();
        }
        let bare = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: bare,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        drop(ws);

        // Reopen: no snapshot (root = None) → full replay, which now
        // defers text.
        let storage = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
        let ws2 = Workspace::open_with_storage(actor, storage, None).unwrap();

        // Tree structure is fully materialized; block text is not.
        assert_eq!(ws2.tree().node_count(), 65);
        assert_eq!(
            ws2.resident_text_count(),
            0,
            "boot must not eagerly materialize any block text"
        );
        assert_eq!(ws2.live_doc_count(), 0);

        // Reading a block never touched since boot rebuilds its text
        // lazily and correctly.
        let (first, first_text) = &edited[0];
        assert_eq!(ws2.block_text(*first).as_deref(), Some(first_text.as_str()));
        assert_eq!(
            ws2.resident_text_count(),
            1,
            "only the read block should now be resident"
        );

        // Every other block reads back byte-identical on demand; a
        // create-only block has no text (not a phantom empty string).
        for (n, want) in &edited {
            assert_eq!(ws2.block_text(*n).as_deref(), Some(want.as_str()));
        }
        assert_eq!(ws2.block_text(bare), None);
        // Lazy reads only ever populate the string cache, never the Doc LRU.
        assert_eq!(ws2.live_doc_count(), 0);
    }

    /// The live-Doc cache never grows past its cap, no matter how many
    /// distinct blocks get edited in a session.
    #[test]
    fn doc_cache_is_bounded() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();

        let over = DOC_CACHE_CAP + 50;
        for i in 0..over {
            let n = NodeId::new();
            ws.apply(make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            ))
            .unwrap();
            let update = ws.build_text_replace_update(n, &format!("b{i}"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: update,
                },
            ))
            .unwrap();
            assert!(ws.live_doc_count() <= DOC_CACHE_CAP);
        }
        assert_eq!(ws.live_doc_count(), DOC_CACHE_CAP);
    }

    /// A block evicted from the cache is rebuilt from the log on the next
    /// edit, preserving its text instead of losing history.
    #[test]
    fn evicted_block_rebuilds_from_log() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();

        let first = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: first,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let update = ws.build_text_replace_update(first, "hello");
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: first,
                text_op: update,
            },
        ))
        .unwrap();

        // Edit enough other blocks to evict `first` from the cache.
        for i in 0..DOC_CACHE_CAP + 10 {
            let n = NodeId::new();
            ws.apply(make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            ))
            .unwrap();
            let u = ws.build_text_replace_update(n, &format!("x{i}"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: u,
                },
            ))
            .unwrap();
        }
        assert!(!ws.content.is_cached(first));

        // Rebuild on demand: appending to the evicted block keeps "hello".
        let update = ws.build_text_replace_update(first, "hello world");
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: first,
                text_op: update,
            },
        ))
        .unwrap();
        assert_eq!(ws.block_text(first).as_deref(), Some("hello world"));
    }
}
