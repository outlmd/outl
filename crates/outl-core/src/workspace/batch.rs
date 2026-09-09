//! Deferred (batched) persistence for [`Workspace`].
//!
//! Composite actions (append a forest, indent a subtree) call
//! [`Workspace::apply`] in a loop. Each op depends on the tree state the
//! previous one mutated, so the ops cannot be collected up front and
//! handed to storage as one `Vec` — the batching has to live *inside*
//! the workspace, deferring only the persist while every op still flows
//! through the CRDT one at a time.
//!
//! [`Workspace::begin_batch`] opens that deferred mode via a
//! [`WorkspaceBatch`] RAII guard. While a guard is live, `apply` runs
//! the full dedup + Yrs merge + CRDT path but buffers the persist
//! instead of fsyncing per op. The outermost guard flushes the buffer
//! with one [`Storage::append_ops`](crate::storage::Storage::append_ops)
//! per destination on `commit` (or, as a best-effort fallback, on drop).
//!
//! Buffered ops are never dropped: they already live in the CRDT and the
//! in-memory log, so discarding them would violate "the op log is the
//! source of truth". A drop without `commit` still flushes.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use tracing::warn;

use super::router::Route;
use super::{Workspace, WorkspaceError};
use crate::op::{op_node, LogOp};

impl Workspace {
    /// Enter deferred-persistence (batch) mode.
    ///
    /// Returns a [`WorkspaceBatch`] guard that derefs to the workspace,
    /// so composite actions pass `&mut *guard` to functions that take
    /// `&mut Workspace`. Every `apply` through the guard mutates the CRDT
    /// and log immediately but buffers its persist; the buffer is flushed
    /// with one `append_ops` per destination when the outermost guard
    /// commits (or drops).
    ///
    /// Batches nest: an inner `begin_batch` bumps a depth counter and only
    /// the outermost guard flushes.
    ///
    /// The buffer only ever carries ops from the **local** actor
    /// (`self.actor`). Composite actions build their ops through the local
    /// [`HlcGenerator`](crate::hlc::HlcGenerator), so every `apply` on a
    /// live guard is a local op; a foreign op (sync replay) is not meant to
    /// flow through a batch. This matters at flush time:
    /// [`JsonlStorage::append_ops`](crate::storage::JsonlStorage) enforces a
    /// foreign-actor guard and rejects the **whole** batch if any op belongs
    /// to another actor, so a stray foreign op would lose the entire flush.
    pub fn begin_batch(&mut self) -> WorkspaceBatch<'_> {
        self.enter_batch();
        WorkspaceBatch {
            ws: self,
            committed: false,
        }
    }

    /// Enter batch mode **without** an RAII guard, for callers that
    /// cannot hold a `&mut Workspace` borrow across the batched region.
    ///
    /// [`Self::begin_batch`] is the guard-based path and should be
    /// preferred. This manual pair exists for a client that batches a
    /// loop while re-borrowing an enclosing context each iteration
    /// (e.g. `outl batch`, whose per-op dispatch needs `&mut WsCtx`, not
    /// `&mut Workspace`, so a guard borrowing the `workspace` field would
    /// deadlock the borrow checker). Every `enter_batch` MUST be paired
    /// with exactly one [`Self::end_batch`]; wrap the pair in the caller's
    /// own RAII type so a panic still flushes.
    pub fn enter_batch(&mut self) {
        self.batch_depth = self.batch_depth.saturating_add(1);
    }

    /// Close a batch opened with [`Self::enter_batch`]. Decrements the
    /// depth and, when the outermost batch closes, flushes the buffer
    /// with one `append_ops` per destination — identical to what an
    /// outermost [`WorkspaceBatch`] does on commit/drop.
    pub fn end_batch(&mut self) -> Result<(), WorkspaceError> {
        self.batch_depth = self.batch_depth.saturating_sub(1);
        if self.batch_depth == 0 {
            self.flush_pending()
        } else {
            Ok(())
        }
    }

    /// Resolve the storage route for `op` against the current tree state.
    /// Called by [`Workspace::apply`] the moment the op is applied, so
    /// page routing reflects the tree the op itself may have mutated.
    pub(super) fn route_for_op(&self, op: &LogOp) -> Route {
        self.router.route(&self.tree, op_node(&op.op))
    }

    /// Run the shared snapshot trigger, counting `applied` new ops.
    ///
    /// The immediate `apply` path calls this with `1`; the batch flush
    /// calls it once with the whole batch size. `snapshot_threshold = 0`
    /// (the CLI) and in-memory workspaces (`snapshots_dir = None`) opt out.
    pub(super) fn trigger_snapshot(&mut self, applied: u32) {
        if self.snapshots.record(applied) {
            self.spawn_background_snapshot();
        }
    }

    /// Flush every buffered op to storage, grouped by destination.
    ///
    /// One [`Storage::append_ops`](crate::storage::Storage::append_ops)
    /// per destination (one fsync each), preserving each op's relative
    /// order within its own destination. Append order within a shard is
    /// meaningful; order across distinct shards is not (they are separate
    /// files). Then the snapshot trigger fires once for the whole batch.
    ///
    /// **Partial failure:** a failure on any destination propagates
    /// immediately; destinations already flushed stay persisted. This is
    /// the same partial-failure surface the per-op `apply` path has today
    /// — the ops all remain in the in-memory log and CRDT regardless, so
    /// nothing is lost, only not-yet-durable.
    pub(super) fn flush_pending(&mut self) -> Result<(), WorkspaceError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        let total = pending.len() as u32;

        // Group by destination so each shard takes one `append_ops`
        // (one fsync), preserving each op's order within its own file.
        let mut grouped: HashMap<Route, Vec<LogOp>> = HashMap::new();
        for (route, op) in pending {
            grouped.entry(route).or_default().push(op);
        }
        for (route, ops) in grouped {
            self.router.append_ops(&route, &ops)?;
        }

        self.trigger_snapshot(total);
        Ok(())
    }
}

/// RAII guard for a deferred-persistence batch.
///
/// Obtained from [`Workspace::begin_batch`]. Derefs to the underlying
/// [`Workspace`], so the guard is a drop-in `&mut Workspace` for the
/// composite actions that build ops one at a time. Buffered persists are
/// flushed when the **outermost** guard commits or drops.
pub struct WorkspaceBatch<'a> {
    ws: &'a mut Workspace,
    committed: bool,
}

impl WorkspaceBatch<'_> {
    /// Close the batch, flushing buffered ops if this is the outermost
    /// guard, and propagate any storage error from the flush.
    ///
    /// A nested (non-outermost) commit only decrements the depth counter
    /// and returns `Ok(())`; the flush happens when the outer guard closes.
    pub fn commit(mut self) -> Result<(), WorkspaceError> {
        // Mark committed before flushing so the subsequent `Drop` (which
        // still runs after `commit` returns) never flushes a second time.
        self.committed = true;
        self.finish()
    }

    /// Decrement the depth; flush the buffer when it reaches zero.
    fn finish(&mut self) -> Result<(), WorkspaceError> {
        self.ws.end_batch()
    }
}

impl Deref for WorkspaceBatch<'_> {
    type Target = Workspace;

    fn deref(&self) -> &Workspace {
        self.ws
    }
}

impl DerefMut for WorkspaceBatch<'_> {
    fn deref_mut(&mut self) -> &mut Workspace {
        self.ws
    }
}

impl Drop for WorkspaceBatch<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // A guard dropped without `commit` still flushes: the ops live in
        // the CRDT + in-memory log already, so dropping them would break
        // "the op log is the source of truth". Best-effort — a storage
        // error can't propagate out of `Drop`, so it is logged.
        if let Err(e) = self.finish() {
            warn!("batch flush on drop failed (ops applied in memory, not persisted): {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use yrs::{Doc, Text, Transact};

    use crate::fractional::Fractional;
    use crate::hlc::{Hlc, HlcGenerator};
    use crate::id::{ActorId, NodeId};
    use crate::op::{LogOp, Op};
    use crate::property::PropValue;
    use crate::storage::{JsonlStorage, MemoryStorage, Storage, StorageError};
    use crate::workspace::Workspace;

    fn make_op(g: &HlcGenerator, op: Op) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    fn create_under(node: NodeId, parent: NodeId) -> Op {
        Op::Create {
            node,
            parent,
            position: Fractional::first(),
        }
    }

    fn create_at(node: NodeId, parent: NodeId, position: &str) -> Op {
        Op::Create {
            node,
            parent,
            position: Fractional::parse(position).expect("valid fractional"),
        }
    }

    /// A `Move` with sentinel `old_*` fields (`do_op` overwrites them).
    fn move_to(node: NodeId, new_parent: NodeId) -> Op {
        Op::Move {
            node,
            new_parent,
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        }
    }

    fn set_prop(node: NodeId, key: &str, value: &str) -> Op {
        Op::SetProp {
            node,
            key: key.to_string(),
            value: Some(PropValue::Text(value.to_string())),
            old_value: None,
        }
    }

    /// A Yrs `update_v1` inserting `s` into a fresh block. Applying the
    /// same bytes to two empty blocks yields identical text, so it can seed
    /// one shared op program delivered to both workspaces below.
    fn text_update(s: &str) -> Vec<u8> {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        text.push(&mut txn, s);
        txn.encode_update_v1()
    }

    /// Storage double that records what actually hit the backend and how
    /// (per-op vs batched), so tests can assert the persist path.
    #[derive(Default)]
    struct Counters {
        append_op: AtomicUsize,
        append_ops: AtomicUsize,
        persisted: Mutex<Vec<LogOp>>,
    }

    struct RecordingStorage {
        inner: MemoryStorage,
        counters: Arc<Counters>,
    }

    impl RecordingStorage {
        fn new() -> (Box<Self>, Arc<Counters>) {
            let counters = Arc::new(Counters::default());
            let store = Box::new(Self {
                inner: MemoryStorage::new(),
                counters: counters.clone(),
            });
            (store, counters)
        }
    }

    impl Storage for RecordingStorage {
        fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError> {
            self.counters.append_op.fetch_add(1, Ordering::SeqCst);
            self.counters
                .persisted
                .lock()
                .expect("lock")
                .push(op.clone());
            self.inner.append_op(op)
        }

        fn append_ops(&mut self, ops: &[LogOp]) -> Result<(), StorageError> {
            self.counters.append_ops.fetch_add(1, Ordering::SeqCst);
            self.counters
                .persisted
                .lock()
                .expect("lock")
                .extend(ops.iter().cloned());
            self.inner.append_ops(ops)
        }

        fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError> {
            self.inner.ops_since(ts)
        }

        fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
            self.inner.ops_for_node(id)
        }

        fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError> {
            self.inner.ops_for_actor(id)
        }

        fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
            self.inner.last_ts_per_actor()
        }

        fn all_ops(&self) -> Result<Vec<LogOp>, StorageError> {
            self.inner.all_ops()
        }
    }

    /// A batch + commit persists exactly the same ops, in the same order,
    /// as N sequential immediate applies.
    #[test]
    fn batch_persists_same_ops_as_sequential() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        // One shared op program so both workspaces persist identical ops.
        let ops: Vec<LogOp> = (0..8)
            .map(|_| make_op(&g, create_under(NodeId::new(), NodeId::root())))
            .collect();

        let (seq_store, seq_counters) = RecordingStorage::new();
        let mut seq = Workspace::open_with_storage(actor, seq_store, None).expect("open seq");
        for op in &ops {
            seq.apply(op.clone()).expect("seq apply");
        }

        let (batch_store, batch_counters) = RecordingStorage::new();
        let mut batched =
            Workspace::open_with_storage(actor, batch_store, None).expect("open batch");
        {
            let mut b = batched.begin_batch();
            for op in &ops {
                b.apply(op.clone()).expect("batch apply");
            }
            b.commit().expect("commit");
        }

        let seq_persisted = seq_counters.persisted.lock().expect("lock").clone();
        let batch_persisted = batch_counters.persisted.lock().expect("lock").clone();
        assert_eq!(seq_persisted, batch_persisted);
        assert_eq!(batch_persisted, ops);
    }

    /// The batch makes exactly one `append_ops` call and zero `append_op`
    /// calls; the immediate path is the mirror image.
    #[test]
    fn batch_calls_append_ops_once_per_destination() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (store, counters) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, store, None).expect("open");
        {
            let mut b = ws.begin_batch();
            for _ in 0..5 {
                b.apply(make_op(&g, create_under(NodeId::new(), NodeId::root())))
                    .expect("apply");
            }
            b.commit().expect("commit");
        }
        assert_eq!(counters.append_ops.load(Ordering::SeqCst), 1);
        assert_eq!(counters.append_op.load(Ordering::SeqCst), 0);

        // Sanity: outside a batch every apply is one `append_op`.
        let (store2, counters2) = RecordingStorage::new();
        let mut ws2 = Workspace::open_with_storage(actor, store2, None).expect("open");
        for _ in 0..5 {
            ws2.apply(make_op(&g, create_under(NodeId::new(), NodeId::root())))
                .expect("apply");
        }
        assert_eq!(counters2.append_op.load(Ordering::SeqCst), 5);
        assert_eq!(counters2.append_ops.load(Ordering::SeqCst), 0);
    }

    /// Ops route to the correct per-page shard (or the global store),
    /// each destination receiving its ops in order.
    #[test]
    fn batch_routes_to_page_shards() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (global_store, global_c) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, global_store, None).expect("open");

        let root_a = NodeId::new();
        let root_b = NodeId::new();
        ws.register_page_root(root_a, "page-a");
        ws.register_page_root(root_b, "page-b");
        let (store_a, count_a) = RecordingStorage::new();
        let (store_b, count_b) = RecordingStorage::new();
        ws.register_page_storage("page-a", store_a);
        ws.register_page_storage("page-b", store_b);

        // Materialize the two page roots first (each routes to its own
        // shard because its own id is registered).
        let root_a_op = make_op(&g, create_under(root_a, NodeId::root()));
        let root_b_op = make_op(&g, create_under(root_b, NodeId::root()));
        // Children + a global (non-page) node.
        let a1 = make_op(&g, create_under(NodeId::new(), root_a));
        let a2 = make_op(&g, create_under(NodeId::new(), root_a));
        let b1 = make_op(&g, create_under(NodeId::new(), root_b));
        let global = make_op(&g, create_under(NodeId::new(), NodeId::root()));

        {
            let mut batch = ws.begin_batch();
            batch.apply(root_a_op.clone()).expect("apply");
            batch.apply(root_b_op.clone()).expect("apply");
            batch.apply(a1.clone()).expect("apply");
            batch.apply(b1.clone()).expect("apply");
            batch.apply(a2.clone()).expect("apply");
            batch.apply(global.clone()).expect("apply");
            batch.commit().expect("commit");
        }

        let a_persisted = count_a.persisted.lock().expect("lock").clone();
        let b_persisted = count_b.persisted.lock().expect("lock").clone();
        let g_persisted = global_c.persisted.lock().expect("lock").clone();

        assert_eq!(a_persisted, vec![root_a_op, a1, a2]);
        assert_eq!(b_persisted, vec![root_b_op, b1]);
        assert_eq!(g_persisted, vec![global]);
        // One batched call per touched destination.
        assert_eq!(count_a.append_ops.load(Ordering::SeqCst), 1);
        assert_eq!(count_b.append_ops.load(Ordering::SeqCst), 1);
        assert_eq!(global_c.append_ops.load(Ordering::SeqCst), 1);
    }

    /// Dropping the guard without calling `commit` still persists the
    /// buffered ops (they already live in the CRDT).
    #[test]
    fn batch_drop_without_commit_still_persists() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (store, counters) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, store, None).expect("open");
        let ops: Vec<LogOp> = (0..4)
            .map(|_| make_op(&g, create_under(NodeId::new(), NodeId::root())))
            .collect();
        {
            let mut b = ws.begin_batch();
            for op in &ops {
                b.apply(op.clone()).expect("apply");
            }
            // No commit — the guard drops here.
        }
        assert_eq!(counters.persisted.lock().expect("lock").clone(), ops);
        assert_eq!(counters.append_ops.load(Ordering::SeqCst), 1);
    }

    /// Nested batches buffer everything until the outermost guard closes,
    /// then flush exactly once.
    #[test]
    fn nested_batches_flush_once_at_outermost() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (store, counters) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, store, None).expect("open");

        let outer = make_op(&g, create_under(NodeId::new(), NodeId::root()));
        let inner = make_op(&g, create_under(NodeId::new(), NodeId::root()));

        {
            let mut b1 = ws.begin_batch();
            b1.apply(outer.clone()).expect("apply");
            {
                let mut b2 = b1.begin_batch();
                b2.apply(inner.clone()).expect("apply");
                b2.commit().expect("inner commit");
                // Inner commit must NOT flush — depth is still 1.
                assert_eq!(counters.persisted.lock().expect("lock").len(), 0);
                assert_eq!(counters.append_ops.load(Ordering::SeqCst), 0);
            }
            // Still nothing persisted after the inner scope.
            assert_eq!(counters.persisted.lock().expect("lock").len(), 0);
            b1.commit().expect("outer commit");
        }

        assert_eq!(
            counters.persisted.lock().expect("lock").clone(),
            vec![outer, inner]
        );
        assert_eq!(counters.append_ops.load(Ordering::SeqCst), 1);
    }

    /// A re-delivered op (ts already in the log) is deduplicated inside a
    /// batch just like outside one: it never reaches the buffer or disk.
    #[test]
    fn batch_skips_duplicate_ops() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (store, counters) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, store, None).expect("open");
        let op = make_op(&g, create_under(NodeId::new(), NodeId::root()));

        {
            let mut b = ws.begin_batch();
            b.apply(op.clone()).expect("apply");
            // Same ts again — must be a no-op on the buffer.
            b.apply(op.clone()).expect("apply dup");
            b.commit().expect("commit");
        }

        assert_eq!(counters.persisted.lock().expect("lock").clone(), vec![op]);
        assert_eq!(counters.append_ops.load(Ordering::SeqCst), 1);
    }

    /// Outside a batch, `apply` persists immediately, one op per
    /// `append_op` call — the pre-batch behaviour.
    #[test]
    fn apply_outside_batch_unchanged() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (store, counters) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, store, None).expect("open");
        let op = make_op(&g, create_under(NodeId::new(), NodeId::root()));
        ws.apply(op.clone()).expect("apply");

        assert_eq!(counters.append_op.load(Ordering::SeqCst), 1);
        assert_eq!(counters.append_ops.load(Ordering::SeqCst), 0);
        assert_eq!(counters.persisted.lock().expect("lock").clone(), vec![op]);
    }

    /// A cycle-forming `Move` inside a batch is a no-op on the materialized
    /// tree (invariant #4) but the op still lands in the in-memory log and
    /// is persisted on commit — the buffer never drops it (invariant #5 /
    /// no-silent-loss).
    #[test]
    fn batch_buffers_cycle_move_and_keeps_it_in_log() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let (store, counters) = RecordingStorage::new();
        let mut ws = Workspace::open_with_storage(actor, store, None).expect("open");

        // A is B's parent; moving A under its own descendant B closes a loop.
        let a = NodeId::new();
        let b = NodeId::new();
        let create_a = make_op(&g, create_under(a, NodeId::root()));
        let create_b = make_op(&g, create_under(b, a));
        let cycle_move = make_op(&g, move_to(a, b));

        {
            let mut batch = ws.begin_batch();
            batch.apply(create_a.clone()).expect("create a");
            batch.apply(create_b.clone()).expect("create b");
            batch.apply(cycle_move.clone()).expect("cycle move");

            // (a) No-op on the materialized tree: A keeps its original
            // parent (root); it did NOT re-parent under its own descendant.
            assert_eq!(batch.tree().parent(a), Some(NodeId::root()));
            assert_eq!(batch.tree().parent(b), Some(a));
            // (b1) The op is already in the in-memory log while still
            // buffered — `apply_op` logged it before the persist deferred.
            assert!(batch.log().contains_ts(&cycle_move.ts));

            batch.commit().expect("commit");
        }

        // (b2) After commit the cycle move is persisted alongside the two
        // creates, in order — a no-op on the tree is never a no-op on the
        // log (invariant #4).
        let persisted = counters.persisted.lock().expect("lock").clone();
        assert_eq!(persisted, vec![create_a, create_b, cycle_move]);
    }

    /// Reopening a JSONL-backed workspace after a batch reproduces the same
    /// materialized state as reopening one persisted sequentially — same
    /// parents, same sibling order (positions), same properties, same block
    /// text. The batch converges like sequential applies down to the leaf
    /// values, not just the parent chain.
    #[test]
    fn reload_after_batch_reproduces_tree() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        // One shared op program: a parent with three ordered children, two
        // properties, and one edited block. Delivered identically to a
        // sequential workspace and a batched one, so everything reload
        // rebuilds from their logs must match.
        let parent = NodeId::new();
        let c0 = NodeId::new();
        let c1 = NodeId::new();
        let c2 = NodeId::new();
        let program: Vec<LogOp> = vec![
            make_op(&g, create_at(parent, NodeId::root(), "m")),
            make_op(&g, create_at(c0, parent, "a")),
            make_op(&g, create_at(c1, parent, "b")),
            make_op(&g, create_at(c2, parent, "c")),
            make_op(&g, set_prop(c0, "status", "done")),
            make_op(&g, set_prop(c1, "color", "blue")),
            make_op(
                &g,
                Op::Edit {
                    node: c2,
                    text_op: text_update("hello outl"),
                },
            ),
        ];
        let nodes = [parent, c0, c1, c2];

        // Persist the program sequentially (one append per op).
        let tmp_seq = tempfile::TempDir::new().expect("tempdir seq");
        {
            let storage = Box::new(
                JsonlStorage::open(tmp_seq.path().to_path_buf(), actor).expect("open seq"),
            );
            let mut ws = Workspace::open_with_storage(actor, storage, None).expect("open seq ws");
            for op in &program {
                ws.apply(op.clone()).expect("seq apply");
            }
        }

        // Persist the exact same program through a batch (one flush).
        let tmp_batch = tempfile::TempDir::new().expect("tempdir batch");
        {
            let storage = Box::new(
                JsonlStorage::open(tmp_batch.path().to_path_buf(), actor).expect("open batch"),
            );
            let mut ws = Workspace::open_with_storage(actor, storage, None).expect("open batch ws");
            let mut b = ws.begin_batch();
            for op in &program {
                b.apply(op.clone()).expect("batch apply");
            }
            b.commit().expect("commit");
        }

        // Reload both from disk (root = None → full replay).
        let seq = Workspace::open_with_storage(
            actor,
            Box::new(JsonlStorage::open(tmp_seq.path().to_path_buf(), actor).expect("reopen seq")),
            None,
        )
        .expect("reopen seq ws");
        let batched = Workspace::open_with_storage(
            actor,
            Box::new(
                JsonlStorage::open(tmp_batch.path().to_path_buf(), actor).expect("reopen batch"),
            ),
            None,
        )
        .expect("reopen batch ws");

        assert_eq!(batched.tree().node_count(), seq.tree().node_count());
        assert_eq!(batched.tree().node_count(), nodes.len());

        for n in nodes {
            assert_eq!(
                batched.tree().parent(n),
                seq.tree().parent(n),
                "parent mismatch for {n}"
            );
            // Sibling order lives in the fractional position.
            assert_eq!(
                batched.tree().position(n),
                seq.tree().position(n),
                "position mismatch for {n}"
            );
            // Properties (sorted by key — iteration order is unspecified).
            let mut seq_props: Vec<(String, PropValue)> = seq
                .tree()
                .properties_of(n)
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect();
            let mut batch_props: Vec<(String, PropValue)> = batched
                .tree()
                .properties_of(n)
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect();
            seq_props.sort_by(|x, y| x.0.cmp(&y.0));
            batch_props.sort_by(|x, y| x.0.cmp(&y.0));
            assert_eq!(batch_props, seq_props, "properties mismatch for {n}");
            // Block text content.
            assert_eq!(
                batched.block_text(n),
                seq.block_text(n),
                "content mismatch for {n}"
            );
        }

        // Spot-check the values actually survived (not just that the two
        // reloads agree with each other).
        assert_eq!(
            batched.tree().position(c0),
            Some(&Fractional::parse("a").expect("valid fractional"))
        );
        assert_eq!(
            batched.tree().property(c0, "status"),
            Some(&PropValue::Text("done".to_string()))
        );
        assert_eq!(batched.block_text(c2).as_deref(), Some("hello outl"));
    }
}
