//! The boot cache, from the `Workspace`'s side.
//!
//! `workspace/snapshot_policy.rs` answers *whether* and *where*; this
//! answers *what goes in* and *who waits for the write*. They are
//! separate files because the policy is testable without building a
//! workspace at all, while everything here needs the tree and the
//! content store.
//!
//! The split from `workspace.rs` is the one `boot.rs` already made:
//! that file answers questions about the document, and this is a
//! projection of the document maintained for exactly one purpose, which
//! is how long the next launch takes.
//!
//! Two forms of the same write, and picking the wrong one is a
//! regression no test will catch:
//!
//! - [`Workspace::refresh_snapshot_in_background`] is what a **boot**
//!   wants. The body is built on the caller's thread (it reads the
//!   tree); the encode, the fsync and the rename move to a worker.
//! - [`Workspace::save_snapshot`] is what a **shutdown** wants. The
//!   caller is about to drop the workspace, so a spawned worker would
//!   race the process exit.
//!
//! Calling the synchronous one in front of a first frame is what cost
//! 306 ms of a cold open on a 10k-page workspace, writing a cache only
//! the next launch reads. See `docs/boot-performance.md`.

use tracing::{debug, warn};

use crate::snapshot::{self, SnapshotBody};
use crate::storage::StorageError;

use super::{Workspace, WorkspaceError};

impl Workspace {
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
        // Both sets: a `stale` node's cached string is the snapshot's,
        // which the delta has already moved past. Writing a body from it
        // would persist text the log disagrees with, and the next boot
        // would adopt that as settled.
        for node in self
            .content
            .pending_snapshot()
            .into_iter()
            .chain(self.content.stale_snapshot())
        {
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
    pub(super) fn spawn_background_snapshot(&mut self) -> bool {
        let body = match self.build_snapshot_body() {
            Ok(Some(b)) => b,
            Ok(None) => return false,
            Err(e) => {
                warn!("skipping background snapshot (cutoff read failed): {e}");
                return false;
            }
        };

        self.snapshots.spawn_write(self.actor, body);
        true
    }

    /// Re-persist the boot cache **off the calling thread**, when
    /// [`Self::snapshot_refresh_due`] says one is owed. Returns whether a
    /// write was started.
    ///
    /// Same decision and the same body as [`Self::save_snapshot`]; what
    /// differs is who waits for the encode, the fsync and the rename.
    ///
    /// This is the form a **boot** wants. `save_snapshot` is synchronous,
    /// so a client that called it before publishing the workspace made
    /// the user wait out a multi-MB serialize and an `F_FULLFSYNC` before
    /// the first frame — 306 ms of a cold open on a 10k-page workspace,
    /// spent writing a cache that only the *next* launch reads. Nothing
    /// on screen depends on it, so nothing on screen should wait for it.
    ///
    /// The body is still built on the caller's thread, because building it
    /// reads the tree and the content store. That part is short and
    /// unavoidable; the part that is neither moves to a worker.
    ///
    /// Not a replacement for `save_snapshot` on **shutdown**: there the
    /// caller is about to drop the workspace, and a spawned worker would
    /// race the process exit.
    pub fn refresh_snapshot_in_background(&mut self) -> bool {
        if !self.snapshot_refresh_due() {
            return false;
        }
        self.spawn_background_snapshot()
    }

    /// Block until every background snapshot worker finishes.
    ///
    /// Call on graceful shutdown so a long-lived client doesn't exit
    /// with a snapshot write still in flight (which would race the
    /// process exit and could leave a stale `.tmp` behind). Errors
    /// inside the workers are already logged; this just joins.
    pub fn wait_for_snapshots(&mut self) {
        self.snapshots.wait();
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
        self.snapshots.set_policy(enabled, threshold);
    }

    /// Should the caller re-persist the snapshot? The rule, and the
    /// regression that bought it, live on `SnapshotPolicy::refresh_due`.
    pub fn snapshot_refresh_due(&self) -> bool {
        self.snapshots
            .refresh_due(self.booted_from_snapshot(), self.log.len())
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
        // The opt-out binds direct callers too: `set_policy(false, ..)`
        // zeroes the threshold but leaves `dir` set, so the dir check
        // below is not one (the TUI's exit flush wrote through it).
        if !self.snapshots.enabled() {
            return Ok(());
        }
        let snapshots_dir = match self.snapshots.dir() {
            Some(d) => d.to_path_buf(),
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
}
