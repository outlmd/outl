//! Per-block text materialization.
//!
//! The structural CRDT (`Tree`) is text-free; block text convergence rides
//! on Yrs. [`ContentStore`] owns that side of a [`Workspace`], and it is
//! deliberately the *only* thing that writes the two text tiers so they
//! can't drift:
//!
//! - `text`: the materialized string of a block, the hot read path.
//!   Cheap, roughly the text size. Populated lazily: the full-replay boot
//!   path does **not** materialize every block up front (that O(all
//!   blocks) pass was a major boot freeze on large snapshotless vaults,
//!   #179). Instead boot records which nodes carry `Edit` history in
//!   `pending`, and [`ContentStore::text`] rebuilds a node's string on
//!   first read. Snapshot boot still hydrates the whole map eagerly (it's
//!   already materialized strings, not a replay).
//! - `pending`: nodes known to have `Edit` ops whose string hasn't been
//!   materialized yet. Membership answers "does this node have text?"
//!   in O(1) so a never-edited node reads back as `None` without a log
//!   scan; a pending node materializes on demand and drops out of the set.
//! - `cache`: a bounded LRU of live Yrs `Doc`s, only for blocks being
//!   edited or merged right now. Rebuilt on demand from the op log.
//!
//! Holding a live `Doc` for every block is what pushed large vaults past
//! the iOS memory limit (issue #108); the materialized string keeps
//! steady-state RAM flat regardless of vault size.
//!
//! `text` and `pending` sit behind [`RefCell`] so `text` (an `&self` read
//! accessor with ~150 call sites, many holding an immutable tree borrow in
//! a loop) can populate the cache on a read without forcing `&mut self` up
//! the whole stack. A `Workspace` is only ever reached through
//! `Arc<Mutex<..>>`, so it needs `Send` (which `RefCell<T: Send>` keeps),
//! never `Sync`; there is no cross-thread sharing of a bare `&ContentStore`
//! to race on.
//!
//! [`Workspace`]: crate::workspace::Workspace
//! [`RefCell`]: std::cell::RefCell

use crate::id::NodeId;
use crate::log::OpLog;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, Text, Transact};

/// How many live Yrs documents stay resident at once.
///
/// Only blocks being actively edited (or merging an incoming update) need
/// a live `Doc`; every read is served from the materialized-text map. A
/// few hundred live docs is a couple of MB, far under the iOS memory
/// limit even on vaults in the hundreds of thousands of blocks.
pub(crate) const DOC_CACHE_CAP: usize = 512;

/// Build a fresh Yrs `Doc` by replaying a block's `Edit` updates.
///
/// Yrs is a CRDT, so the order updates are applied in does not change the
/// resulting document. Malformed updates (corrupted peers) are skipped.
fn build_doc<'a>(updates: impl Iterator<Item = &'a [u8]>) -> Doc {
    let doc = Doc::new();
    {
        let _text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        for update in updates {
            if let Ok(decoded) = yrs::Update::decode_v1(update) {
                let _ = txn.apply_update(decoded);
            }
        }
    }
    doc
}

/// Replay `updates` one at a time, capturing the block's text after each
/// one. Entry `i` is the block's text once updates `0..=i` are applied, so
/// the returned vector is aligned 1:1 with the `Op::Edit`s it was built
/// from and its last entry equals [`build_doc`] + [`doc_string`] over the
/// same input.
///
/// This is what makes an `Op::Edit` that *shrank* a block recoverable: the
/// pre-edit text is not stored anywhere as text, but every `Edit` carries
/// the Yrs delta that produced it, so replaying a prefix of the history
/// reconstructs the state the block was in before the shrink.
///
/// Order matters here in a way it does not for [`build_doc`]. Yrs is a
/// CRDT, so the *final* string is order-independent; the intermediate ones
/// are not, and are only meaningful when the caller feeds updates in HLC
/// order — the order the ops actually happened in.
///
/// A malformed update still contributes an entry (the unchanged text)
/// rather than being dropped, so a caller can map entry `i` back to the
/// op it came from.
pub(crate) fn text_revisions<'a>(updates: impl Iterator<Item = &'a [u8]>) -> Vec<String> {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    let mut out = Vec::new();
    for update in updates {
        let mut txn = doc.transact_mut();
        if let Ok(decoded) = yrs::Update::decode_v1(update) {
            let _ = txn.apply_update(decoded);
        }
        out.push(text.get_string(&txn));
    }
    out
}

/// Materialize a `Doc`'s `content` text into a plain string.
fn doc_string(doc: &Doc) -> String {
    let text = doc.get_or_insert_text("content");
    let txn = doc.transact();
    text.get_string(&txn)
}

/// Rebuild a node's `Doc` by replaying its `Edit` updates from `log`.
///
/// **Routes through [`OpLog::edit_updates`], never a log scan.** That
/// distinction is the whole point of this function existing: the index
/// makes it `O(edits-of-node)` where the obvious
/// `log.iter().filter_map(..)` is `O(total ops)` — 217,811 on the
/// maintainer's workspace, per block.
///
/// This is the **single** owner of "replay one block's history into a
/// `Doc`". It used to be written out twice, in
/// [`materialize_text_from_log`] and in [`ContentStore::ensure_doc`], and
/// only the first was ever fixed — see the note on `ensure_doc`.
///
/// Yrs is a CRDT, so the result is byte-identical regardless of replay
/// order (#179). `log` must carry the node's complete `Edit` history; the
/// full-replay boot path guarantees that (`log_complete == true`), and
/// the snapshot-boot path routes through `Workspace::ensure_doc_for_edit`
/// instead, which loads the history from storage first.
fn doc_from_log(node: NodeId, log: &OpLog) -> Doc {
    build_doc(log.edit_updates(node))
}

/// Rebuild a node's block text by replaying its `Edit` updates from `log`.
fn materialize_text_from_log(node: NodeId, log: &OpLog) -> String {
    doc_string(&doc_from_log(node, log))
}

/// Bounded LRU of live Yrs documents.
///
/// A `Doc` is only needed to *mutate* a block (encode an edit delta) or to
/// merge an incoming remote update. Reads come from the materialized
/// string instead, so at most [`DOC_CACHE_CAP`] docs stay alive; a cold
/// block being edited is rebuilt on demand from the op log.
struct DocCache {
    docs: HashMap<NodeId, Doc>,
    /// Recency queue; the front is the least-recently-used node.
    order: VecDeque<NodeId>,
    cap: usize,
}

impl DocCache {
    fn new(cap: usize) -> Self {
        Self {
            docs: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn contains(&self, node: NodeId) -> bool {
        self.docs.contains_key(&node)
    }

    fn touch(&mut self, node: NodeId) {
        if let Some(pos) = self.order.iter().position(|n| *n == node) {
            self.order.remove(pos);
        }
        self.order.push_back(node);
    }

    fn get_mut(&mut self, node: NodeId) -> Option<&mut Doc> {
        if self.docs.contains_key(&node) {
            self.touch(node);
            self.docs.get_mut(&node)
        } else {
            None
        }
    }

    fn insert(&mut self, node: NodeId, doc: Doc) {
        if !self.docs.contains_key(&node) && self.docs.len() >= self.cap {
            if let Some(evicted) = self.order.pop_front() {
                self.docs.remove(&evicted);
            }
        }
        self.docs.insert(node, doc);
        self.touch(node);
    }
}

/// Per-node block text, the two-tier store behind `Workspace`'s text API.
pub(crate) struct ContentStore {
    /// Materialized block strings. Lazily populated on read (see the
    /// module docs); `RefCell` so the `&self` read path can cache.
    text: RefCell<HashMap<NodeId, String>>,
    cache: DocCache,
    /// Nodes with `Edit` history whose string hasn't been materialized
    /// yet (deferred by the full-replay boot path). Drained on read.
    pending: RefCell<HashSet<NodeId>>,
}

impl Default for ContentStore {
    fn default() -> Self {
        Self {
            text: RefCell::new(HashMap::new()),
            cache: DocCache::new(DOC_CACHE_CAP),
            pending: RefCell::new(HashSet::new()),
        }
    }
}

impl ContentStore {
    /// Materialized text of a block, if it has any.
    ///
    /// The cached string for `node`, if already resident (snapshot body or a
    /// prior read). Cheap `&self` cache probe — no log scan, no materialize.
    pub(crate) fn cached(&self, node: NodeId) -> Option<String> {
        self.text.borrow().get(&node).cloned()
    }

    /// Whether `node` was recorded as carrying `Edit` history at boot but
    /// hasn't been materialized yet. A non-pending, non-cached node never had
    /// any `Edit` (reads back as `None`).
    pub(crate) fn is_pending(&self, node: NodeId) -> bool {
        self.pending.borrow().contains(&node)
    }

    /// Snapshot of the still-deferred nodes, so a caller can materialize each
    /// via the per-node index (`Workspace::force_materialize_pending`) before
    /// building a snapshot — avoiding `materialized_text`'s O(log)-per-block
    /// fallback.
    pub(crate) fn pending_snapshot(&self) -> Vec<NodeId> {
        self.pending.borrow().iter().copied().collect()
    }

    /// Materialize a pending node's string from its `Edit` updates (resolved
    /// by the caller from the per-node INDEX, `Storage::ops_for_node`), cache
    /// it, and drop it from `pending`. The `&self` counterpart of
    /// [`Self::text`] that does NOT scan the log: O(edits of the node), not
    /// O(whole log) per block — the difference between a cheap open and a
    /// pathological one on a full-replay boot of a 200k-op vault.
    pub(crate) fn text_from_edits<'a>(
        &self,
        node: NodeId,
        updates: impl Iterator<Item = &'a [u8]>,
    ) -> String {
        let s = doc_string(&build_doc(updates));
        self.text.borrow_mut().insert(node, s.clone());
        self.pending.borrow_mut().remove(&node);
        s
    }

    /// Record the set of nodes that carry `Edit` history but haven't been
    /// materialized yet. The full-replay boot path calls this instead of
    /// eagerly building every block's `Doc` (#179); [`Self::text`] then
    /// rebuilds each string on first read.
    pub(crate) fn set_pending(&mut self, nodes: HashSet<NodeId>) {
        *self.pending.borrow_mut() = nodes;
    }

    /// Materialize a node's text from its `Edit` updates without caching
    /// the `Doc`. Used during open so the memory peak stays at a single
    /// live doc instead of one per block.
    pub(crate) fn materialize<'a>(
        &mut self,
        node: NodeId,
        updates: impl Iterator<Item = &'a [u8]>,
    ) {
        let doc = build_doc(updates);
        self.text.borrow_mut().insert(node, doc_string(&doc));
        self.pending.borrow_mut().remove(&node);
    }

    /// Ensure `node`'s `Doc` is resident in the cache, rebuilding it from
    /// the block's `Edit` ops in `log` if it was evicted (or never loaded).
    /// No-op when already cached.
    /// # This function is why the bug class needs a guard
    ///
    /// It used to scan the **whole** log (`log.iter().filter_map(..)`) to
    /// find one node's edits. Its comment justified that by avoiding a
    /// transient `Vec<Vec<u8>>` clone — true, and beside the point: the
    /// cost was the `O(total ops)` scan, not the allocation.
    ///
    /// `block_text` had the identical defect and was fixed in #179 by
    /// routing through [`OpLog::edit_updates`]. This sibling was not,
    /// so the first keystroke on any cold block after a full-replay boot
    /// paid a scan of all 217,811 ops on the maintainer's workspace —
    /// on the foreground thread, per block.
    ///
    /// Both now go through [`doc_from_log`], which is the single owner.
    /// A fix that leaves the sibling armed is root `CLAUDE.md`
    /// invariant 9's general rule in miniature: the problem moves, it
    /// does not leave.
    fn ensure_doc(&mut self, node: NodeId, log: &OpLog) {
        if self.cache.contains(node) {
            return;
        }
        self.cache.insert(node, doc_from_log(node, log));
    }

    /// Merge a raw Yrs update into a node's live `Doc` and refresh its
    /// materialized string. Rebuilds the doc from `log` first if it was
    /// cold. Both tiers stay in sync because only this method writes them.
    pub(crate) fn merge_update(&mut self, node: NodeId, log: &OpLog, update: &[u8]) {
        self.ensure_doc(node, log);
        if let Some(doc) = self.cache.get_mut(node) {
            {
                let _text = doc.get_or_insert_text("content");
                let mut txn = doc.transact_mut();
                if let Ok(decoded) = yrs::Update::decode_v1(update) {
                    let _ = txn.apply_update(decoded);
                }
            }
            self.text.borrow_mut().insert(node, doc_string(doc));
            // The node now has a resident, up-to-date string; drop any
            // deferred-materialization marker so it can't linger (symmetry
            // with `materialize`/`replace_text`/`cache_doc`).
            self.pending.borrow_mut().remove(&node);
        }
    }

    /// Rewrite a node's text to `new_text` and return the Yrs delta that
    /// encodes the change. Rebuilds the doc from `log` first if it was
    /// cold, and keeps the materialized string in sync.
    pub(crate) fn replace_text(&mut self, node: NodeId, log: &OpLog, new_text: &str) -> Vec<u8> {
        self.ensure_doc(node, log);
        // `ensure_doc` just cached this node, so the lookup can't miss. If
        // it ever did, returning an empty update would silently drop the
        // edit, so fail loud instead.
        let doc = self
            .cache
            .get_mut(node)
            .expect("ensure_doc guarantees the node's doc is cached");
        let text = doc.get_or_insert_text("content");

        // Snapshot the state vector before our mutation so we can encode
        // only the resulting delta.
        let sv_before = {
            let txn = doc.transact();
            txn.state_vector()
        };

        {
            let mut txn = doc.transact_mut();
            let len = text.len(&txn);
            if len > 0 {
                text.remove_range(&mut txn, 0, len);
            }
            if !new_text.is_empty() {
                text.insert(&mut txn, 0, new_text);
            }
        }

        let update = {
            let txn = doc.transact();
            txn.encode_state_as_update_v1(&sv_before)
        };
        self.text.borrow_mut().insert(node, new_text.to_string());
        self.pending.borrow_mut().remove(&node);
        update
    }

    /// Number of live `Doc`s currently resident. Test-only window into the
    /// bound that keeps large vaults under the iOS memory limit (#108).
    #[cfg(test)]
    pub(crate) fn live_doc_count(&self) -> usize {
        self.cache.docs.len()
    }

    /// Number of block strings currently materialized. Window into the
    /// lazy read path (#179): after a full-replay boot this is `0` until
    /// the first `block_text` read. Surfaced through
    /// `Workspace::resident_text_count` so downstream crates can assert a
    /// read path doesn't force the whole workspace to materialize.
    pub(crate) fn resident_text_count(&self) -> usize {
        self.text.borrow().len()
    }

    /// Whether `node`'s `Doc` is currently resident in the cache.
    pub(crate) fn is_cached(&self, node: NodeId) -> bool {
        self.cache.contains(node)
    }

    /// Build and cache a `Doc` from the given updates, also refreshing
    /// the materialized text. Used when the in-memory log is incomplete
    /// (snapshot boot) and the full `Edit` history must be loaded from
    /// storage to rebuild a correct Doc (#129).
    pub(crate) fn cache_doc<'a>(&mut self, node: NodeId, updates: impl Iterator<Item = &'a [u8]>) {
        if self.cache.contains(node) {
            return;
        }
        let doc = build_doc(updates);
        let s = doc_string(&doc);
        self.text.borrow_mut().insert(node, s);
        self.pending.borrow_mut().remove(&node);
        self.cache.insert(node, doc);
    }

    /// Fully materialize every deferred (`pending`) block and return a
    /// clone of the complete text map. The snapshot writer serializes
    /// this so a snapshot-opened workspace can serve reads without
    /// replaying any `Edit` op — which means the map must carry **every**
    /// block's string, including ones the lazy read path hasn't touched
    /// yet (#179). This is the one place the deferred O(all blocks) work
    /// is paid; it runs off the boot hot path (snapshot build), once, and
    /// makes every subsequent boot a fast snapshot boot.
    pub(crate) fn materialized_text(&self, log: &OpLog) -> HashMap<NodeId, String> {
        let pending: Vec<NodeId> = self.pending.borrow().iter().copied().collect();
        for node in pending {
            // A node already resident (e.g. edited via `apply` after boot)
            // wins over a replay; just drop it from `pending`.
            if !self.text.borrow().contains_key(&node) {
                let s = materialize_text_from_log(node, log);
                self.text.borrow_mut().insert(node, s);
            }
            self.pending.borrow_mut().remove(&node);
        }
        self.text.borrow().clone()
    }

    /// Rebuild a `ContentStore` from a pre-materialized text map. The
    /// LRU `Doc` cache starts empty; cold blocks rebuild on demand from
    /// the op log via `ensure_doc`. `pending` starts empty — the snapshot
    /// map already carries every block's string, so no lazy materialization
    /// is needed on this boot path. Used by the snapshot boot path.
    pub(crate) fn from_text_map(text: HashMap<NodeId, String>) -> Self {
        Self {
            text: RefCell::new(text),
            cache: DocCache::new(DOC_CACHE_CAP),
            pending: RefCell::new(HashSet::new()),
        }
    }
}
