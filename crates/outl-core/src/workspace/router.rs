//! Which storage owns an op, and how the shards read back as one log.
//!
//! Root `CLAUDE.md` invariant 5 says storage is a trait, and invariant 7
//! says convergent state rides the op log with one
//! `ops-<actor>.jsonl` per device. RFC #137 Phase B added a second axis
//! to that: a workspace can shard its log **per page** as well as per
//! actor, so a 10k-page graph does not funnel every op through one file.
//!
//! The cost of the second axis is a routing decision on every write and
//! a merge on every read, and both used to live as three loose fields on
//! [`Workspace`](super::Workspace) — `storage`, `page_storages`,
//! `page_root_to_slug` — with the merge logic repeated four times in
//! four private methods that differed only in which `Storage` call they
//! fanned out.
//!
//! Collecting it here makes the routing rule one function with a name
//! ([`Self::slug_for_node`]), makes the four merges one generic
//! ([`Self::merged`]), and makes both testable against
//! [`MemoryStorage`](crate::storage::MemoryStorage) without building a
//! workspace.
//!
//! # The ordering rule
//!
//! Every merged read sorts by HLC and de-duplicates on it. That is not
//! an optimisation: an op can legitimately appear in two shards (a page
//! registered after some of its ops were written globally), and the CRDT
//! is idempotent per op but the *log* is not allowed to double-count.
//! Sorting by `ts` also means the merge reproduces the exact order a
//! single-file log would have had, which is what makes sharding
//! invisible to replay.

use std::collections::{BTreeMap, HashMap};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::LogOp;
use crate::storage::{Storage, StorageError};
use crate::tree::Tree;

/// Where one op is persisted, resolved **at apply time**.
///
/// Page routing walks the parent chain of the op's node, and that node
/// may be one the same op just created or moved, so the route has to be
/// captured while the tree is in the exact state `apply` left it —
/// re-resolving it at flush time could route to a different shard. That
/// is why the batch path buffers `(Route, LogOp)` pairs rather than
/// re-deriving the destination on flush.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Route {
    /// The legacy single-file-per-actor storage.
    Global,
    /// A registered per-page shard.
    Page(String),
}

/// The global storage plus any per-page shards, and the map that decides
/// between them.
pub(crate) struct StorageRouter {
    /// Global scope — the legacy single-file-per-actor layout.
    global: Box<dyn Storage>,
    /// Per-page shards, keyed by page slug. Empty for a workspace that
    /// has not migrated.
    pages: HashMap<String, Box<dyn Storage>>,
    /// `NodeId → slug` for page roots, populated by the client (which
    /// reads sidecars via `outl-md`). The routing walks an op's node up
    /// its parent chain until it hits one of these.
    root_to_slug: HashMap<NodeId, String>,
}

impl StorageRouter {
    pub(crate) fn new(global: Box<dyn Storage>) -> Self {
        StorageRouter {
            global,
            pages: HashMap::new(),
            root_to_slug: HashMap::new(),
        }
    }

    /// Whether any per-page shard has been registered.
    pub(crate) fn has_pages(&self) -> bool {
        !self.pages.is_empty()
    }

    /// Register a per-page shard. Ops whose node resolves to `slug` are
    /// routed here instead of to the global storage.
    pub(crate) fn register_page(&mut self, slug: &str, storage: Box<dyn Storage>) {
        self.pages.insert(slug.to_string(), storage);
    }

    /// Register a page root → slug mapping, for every page the client
    /// knows about.
    pub(crate) fn register_root(&mut self, root: NodeId, slug: &str) {
        self.root_to_slug.insert(root, slug.to_string());
    }

    /// Walk `tree.parent(node)` up until a registered page root, and
    /// return its slug.
    ///
    /// `None` when the chain dead-ends at the workspace root without
    /// finding one — an unregistered or not-yet-materialized node — so
    /// callers treat it as best-effort.
    pub(crate) fn slug_for_node(&self, tree: &Tree, node: NodeId) -> Option<String> {
        let mut current = node;
        loop {
            if let Some(slug) = self.root_to_slug.get(&current) {
                return Some(slug.clone());
            }
            current = tree.parent(current)?;
        }
    }

    /// Where an op naming `node` should be persisted.
    ///
    /// A slug that resolves but has no registered shard falls back to
    /// [`Route::Global`]: the page is known, its shard is not open, and
    /// dropping the op would be worse than writing it to the file the
    /// merge reads anyway.
    pub(crate) fn route(&self, tree: &Tree, node: Option<NodeId>) -> Route {
        match node.and_then(|n| self.slug_for_node(tree, n)) {
            Some(slug) if self.pages.contains_key(&slug) => Route::Page(slug),
            _ => Route::Global,
        }
    }

    /// Persist one op on `route`.
    pub(crate) fn append_op(&mut self, route: &Route, op: &LogOp) -> Result<(), StorageError> {
        match route {
            Route::Page(slug) => match self.pages.get_mut(slug) {
                Some(s) => s.append_op(op),
                // The shard was unregistered between routing and write.
                // The global file is what the merge reads too, so this
                // keeps the op rather than losing it.
                None => self.global.append_op(op),
            },
            Route::Global => self.global.append_op(op),
        }
    }

    /// Persist a batch on `route`, in one call to the backend.
    pub(crate) fn append_ops(&mut self, route: &Route, ops: &[LogOp]) -> Result<(), StorageError> {
        match route {
            Route::Page(slug) => match self.pages.get_mut(slug) {
                Some(s) => s.append_ops(ops),
                None => self.global.append_ops(ops),
            },
            Route::Global => self.global.append_ops(ops),
        }
    }

    /// Every op across every shard, HLC-ordered and de-duplicated.
    pub(crate) fn all_ops(&self) -> Result<Vec<LogOp>, StorageError> {
        self.merged(|s| s.all_ops())
    }

    /// The per-actor delta across every shard (see
    /// [`Storage::ops_since_per_actor`]), HLC-ordered.
    pub(crate) fn ops_since_per_actor(
        &self,
        cutoff: &BTreeMap<ActorId, Hlc>,
    ) -> Result<Vec<LogOp>, StorageError> {
        self.merged(|s| s.ops_since_per_actor(cutoff))
    }

    /// Every op naming `node`, across every shard, HLC-ordered.
    pub(crate) fn ops_for_node(&self, node: NodeId) -> Result<Vec<LogOp>, StorageError> {
        self.merged(|s| s.ops_for_node(node))
    }

    /// Read from every shard, then order and de-duplicate by HLC.
    ///
    /// The one place the merge rule lives. It used to be copied into
    /// three separate methods that differed only in the closure.
    fn merged<F>(&self, read: F) -> Result<Vec<LogOp>, StorageError>
    where
        F: Fn(&dyn Storage) -> Result<Vec<LogOp>, StorageError>,
    {
        let mut all = read(self.global.as_ref())?;
        for s in self.pages.values() {
            all.extend(read(s.as_ref())?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// The per-actor high-water mark across every shard, used as the
    /// snapshot cutoff. Keeps the **latest** timestamp per actor, since
    /// an actor writes into more than one shard.
    pub(crate) fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
        let mut map = self.global.last_ts_per_actor()?;
        for s in self.pages.values() {
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

    /// Apply the resident-op LRU cap to every shard.
    pub(crate) fn resize_cache(&mut self, cap: usize) {
        self.global.resize_cache(cap);
        for s in self.pages.values_mut() {
            s.resize_cache(cap);
        }
    }
}

#[cfg(test)]
mod tests;
