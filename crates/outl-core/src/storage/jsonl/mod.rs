//! Filesystem-backed op log stored as line-delimited JSON.
//!
//! Each device only writes to its own `ops-<actor>.jsonl` file. Reads
//! merge every file in the directory, so peers see each other's ops as
//! the filesystem layer (iCloud Drive, Syncthing, a shared NFS, etc.)
//! syncs files in. This sidesteps filesystem-level conflicts entirely:
//! the CRDT does its job on the merged view.
//!
//! Layout inside `ops_dir`:
//!
//! ```text
//! <ops_dir>/
//! ├── ops-<this_actor>.jsonl    ← we only ever write here
//! ├── ops-<this_actor>.idx      ← per-actor HLC → offset index (RFC #137)
//! ├── ops-<peer_actor>.jsonl    ← read-only mirrors of other devices
//! └── ...
//! ```
//!
//! ## Memory: bounded LRU + offset index (RFC #137 Phase A)
//!
//! `cache` is a bounded [`LruCache<Hlc, LogOp>`]: the most recently
//! applied ops stay in RAM, older ones are evicted and read back from
//! disk on demand via the offset index. This keeps RSS roughly constant
//! regardless of how much history the workspace has accumulated.
//! `JsonlStorage::open` keeps the legacy "unbounded" default (so
//! existing callers behave byte-for-byte the same); new callers wire
//! a cap through [`JsonlStorage::open_with_cap`] from `[storage] lru_cap`.

mod append;
mod census;
mod read;
#[cfg(test)]
mod read_robustness;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::num::NonZero;
use std::path::PathBuf;

use lru::LruCache;
use parking_lot::RwLock;
use serde::Deserialize;

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{ActorIndex, ActorNodeIndex, PageScope, Storage, StorageError};

/// One-file-per-actor JSONL op log on the filesystem.
///
/// The on-disk layout depends on the [`PageScope`] the storage was
/// opened with:
///
/// - `PageScope::Global` (legacy): `ops/ops-<actor>.jsonl`
/// - `PageScope::PerPage(slug)`: `ops/<actor>/<slug>.jsonl`
///
/// Sidecars (`.idx`, `.nodes.idx`) live next to the `.jsonl` in both
/// layouts. See `own_ops_path` for the exact routing.
pub struct JsonlStorage {
    /// Directory containing every per-actor ops file (Global) or every
    /// per-actor subdirectory (PerPage).
    ops_dir: PathBuf,
    /// This device's actor id; we never write into another actor's file.
    actor: ActorId,
    /// Scope this storage is responsible for. Determines path layout.
    scope: PageScope,
    /// Bounded LRU: hot ops in RAM. Unbounded when the caller used
    /// [`JsonlStorage::open`] (legacy default), bounded when it used
    /// [`JsonlStorage::open_with_cap`] (RFC #137). Cold ops are read
    /// back from the `.jsonl` via [`Self::read_op_at`].
    cache: RwLock<LruCache<Hlc, LogOp>>,
    /// Per-actor HLC offset index — `HLC → byte offset in .jsonl`.
    /// Pure cache; rebuilt on boot if the sidecar `.idx` is missing
    /// or stale. RFC #137.
    index: ActorIndex,
    /// Per-actor secondary index — `NodeId → Vec<(Hlc, offset)>`.
    /// Powers `ops_for_node` without scanning the whole file. Same
    /// cache semantics as `index`. RFC #137 Phase A.
    node_index: ActorNodeIndex,
    /// When `false`, [`Self::append_ops_inner`] refuses before it
    /// opens the `.jsonl`. A read-only open sets this after boot so
    /// the reload can still refresh index sidecars, and an append
    /// cannot create an actor file this process does not lock.
    writes_allowed: bool,
}

impl JsonlStorage {
    /// Open the storage rooted at `ops_dir` for the given `actor`, with
    /// the legacy unbounded cache and `PageScope::Global`. The directory
    /// is created if missing. The merged op log is loaded into memory
    /// on open.
    ///
    /// Equivalent to [`Self::open_with_cap`] with `cap = 0` (unbounded).
    /// New callers should wire `[storage] lru_cap` from `outl.toml`
    /// through [`Self::open_with_cap`] instead.
    pub fn open(ops_dir: PathBuf, actor: ActorId) -> Result<Self, StorageError> {
        Self::open_with_cap(ops_dir, actor, 0)
    }

    /// Open with a bounded LRU cache under `PageScope::Global`. `cap = 0`
    /// means unbounded (the legacy default).
    pub fn open_with_cap(
        ops_dir: PathBuf,
        actor: ActorId,
        cap: usize,
    ) -> Result<Self, StorageError> {
        Self::open_with_scope_cap(ops_dir, actor, PageScope::Global, cap)
    }

    /// Open with explicit scope and LRU cap. This is the constructor
    /// Phase B (RFC #137) wires through `outl init --scope=per-page`
    /// and `outl migrate-to-per-page-ops`.
    ///
    /// Layout:
    /// - `PageScope::Global` → `ops/ops-<actor>.jsonl` (legacy)
    /// - `PageScope::PerPage(slug)` → `ops/<actor>/<slug>.jsonl`
    pub fn open_with_scope_cap(
        ops_dir: PathBuf,
        actor: ActorId,
        scope: PageScope,
        cap: usize,
    ) -> Result<Self, StorageError> {
        // Make sure the directory that will hold the .jsonl exists.
        // For Global that's `ops_dir` itself; for PerPage it's
        // `ops_dir/<actor>/`.
        let own_dir = match &scope {
            PageScope::Global => ops_dir.clone(),
            PageScope::PerPage(_) => ops_dir.join(actor.to_string()),
        };
        std::fs::create_dir_all(&own_dir)
            .map_err(|e| StorageError::Backend(format!("create ops dir: {e}")))?;

        let cache = if cap == 0 {
            LruCache::unbounded()
        } else {
            LruCache::new(NonZero::new(cap).expect("cap > 0"))
        };

        let mut storage = Self {
            ops_dir,
            actor,
            scope,
            cache: RwLock::new(cache),
            index: ActorIndex::new(),
            node_index: ActorNodeIndex::new(),
            writes_allowed: true,
        };
        storage.reload()?;
        Ok(storage)
    }

    fn own_ops_path(&self) -> PathBuf {
        match &self.scope {
            PageScope::Global => self.ops_dir.join(format!("ops-{}.jsonl", self.actor)),
            PageScope::PerPage(slug) => self
                .ops_dir
                .join(format!("{actor}", actor = self.actor))
                .join(format!("{slug}.jsonl")),
        }
    }

    fn own_ops_dir(&self) -> PathBuf {
        match &self.scope {
            PageScope::Global => self.ops_dir.clone(),
            PageScope::PerPage(_) => self.ops_dir.join(self.actor.to_string()),
        }
    }

    /// Directory the storage reads/writes from. Lets clients log it.
    pub fn ops_dir(&self) -> &std::path::Path {
        &self.ops_dir
    }

    /// Scope this storage was opened with.
    pub fn scope(&self) -> &PageScope {
        &self.scope
    }

    /// Per-file diagnostics from the most recent `reload`. Useful for
    /// embedding inside debug snapshots without rerunning the parse.
    pub fn file_stats(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (_, op) in self.cache.read().iter() {
            *counts.entry(format!("ops-{}.jsonl", op.actor)).or_insert(0) += 1;
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Replace the in-memory LRU with one sized `cap`, evicting the
    /// least-recently-inserted ops first. `cap = 0` switches to an
    /// unbounded cache.
    ///
    /// `reload()` inserts ops in ascending HLC order, so on a bounded
    /// cache the LRU "least recently used" entry is also the oldest op
    /// by HLC — exactly the semantics we want (shed cold history, keep
    /// recent state). Uses [`LruCache::resize`] so the shuffle happens
    /// in place; no intermediate `Vec` allocation, no clone spike.
    pub fn resize_cache(&self, cap: usize) {
        let mut guard = self.cache.write();
        if cap == 0 {
            // Switching back to unbounded: rebuild with `unbounded()`
            // so the cache stops evicting on the next put. Same drain +
            // refill shape as the bounded case below, just without a
            // cap.
            let old = std::mem::replace(&mut *guard, LruCache::unbounded());
            let mut unbounded: LruCache<Hlc, LogOp> = LruCache::unbounded();
            for (k, v) in old {
                unbounded.put(k, v);
            }
            *guard = unbounded;
            return;
        }
        let new_cap = NonZero::new(cap).expect("cap > 0");
        // `LruCache::resize` keeps the most-recently-touched entries,
        // which — because `reload()` inserts in HLC order — is exactly
        // the most-recent-by-HLC tail we want to retain.
        guard.resize(new_cap);
    }
}

/// Parse one JSONL line into one-or-more [`LogOp`]s.
///
/// The common case is a single op per line. But a non-atomic, unsynchronized
/// concurrent append (two writers' `write_all`s interleaving on the same file)
/// can glue two ops together with no separating newline — `…}}}{"ts":…` — and
/// sometimes leaves a trailing empty line. Rather than drop the whole line
/// (losing real ops the user authored), we stream every concatenated JSON value
/// off the line via [`serde_json::StreamDeserializer`] and recover all of them.
///
/// `StreamDeserializer` reads consecutive self-delimiting JSON values from one
/// buffer, so `{…}{…}` yields two ops; a clean single-op line yields one. The
/// op log dedups by op id on apply, so recovering a value that another file
/// also carries is harmless.
pub(super) fn parse_log_line(raw: &str) -> Result<Vec<LogOp>, serde_json::Error> {
    let mut ops = Vec::new();
    let stream = serde_json::Deserializer::from_str(raw).into_iter::<LogOp>();
    for item in stream {
        ops.push(item?);
    }
    Ok(ops)
}

/// Parse-lite view of a `LogOp`: only the two fields the boot-time index
/// build needs — the HLC and the node the op touches — leaving the heavy
/// payload (`Op::Edit`'s `text_op` byte array above all) undeserialized.
///
/// serde ignores every field not named here by default, so `text_op`,
/// `position`, `new_parent`, `value`, … are walked-and-dropped instead
/// of allocated. Skipping the `Vec<u8>` allocation per `Edit` is what
/// turns `reload` from an O(log size, allocating) reparse — seconds and
/// a memory spike on a 150 MB / 200k-op log — into an index build that
/// touches the same bytes without allocating them a second time.
#[derive(Deserialize)]
struct LiteLogOp {
    ts: Hlc,
    op: LiteOp,
}

/// Externally-tagged mirror of [`Op`] carrying only each variant's
/// `node`. The variant names must match [`Op`] exactly so serde routes
/// the same JSON; every other field of each variant is an ignored key.
#[derive(Deserialize)]
enum LiteOp {
    Move { node: NodeId },
    Edit { node: NodeId },
    SetProp { node: NodeId },
    Create { node: NodeId },
    SetCollapsed { node: NodeId },
    SnoozeRemind { node: NodeId },
}

impl LiteOp {
    /// The node this op touches. Every [`Op`] variant carries one.
    fn node(&self) -> NodeId {
        match self {
            LiteOp::Move { node }
            | LiteOp::Edit { node }
            | LiteOp::SetProp { node }
            | LiteOp::Create { node }
            | LiteOp::SetCollapsed { node }
            | LiteOp::SnoozeRemind { node } => *node,
        }
    }
}

/// Parse-lite sibling of [`parse_log_line`]: extract only `(ts, node)`
/// per op via [`LiteLogOp`], streaming the same concatenated-JSON
/// recovery so a glued line yields every op's `(ts, node)` instead of
/// dropping the line. Returns the same `Result` shape as
/// [`parse_log_line`] so the record framing (skip-on-parse-error, offset
/// accounting) stays byte-identical to the full path. `node` is always
/// `Some` today (every `Op` targets a node) but kept `Option` to mirror
/// [`op_node`].
pub(super) fn parse_log_line_lite(
    raw: &str,
) -> Result<Vec<(Hlc, Option<NodeId>)>, serde_json::Error> {
    let mut out = Vec::new();
    let stream = serde_json::Deserializer::from_str(raw).into_iter::<LiteLogOp>();
    for item in stream {
        let lite = item?;
        out.push((lite.ts, Some(lite.op.node())));
    }
    Ok(out)
}

/// One physical record read from an op-log file, tolerant of the ways a
/// synced or crash-interrupted `.jsonl` can be malformed.
///
/// Reload only needs each op's `(ts, node)` to build the offset/node
/// indexes, so `Ops` carries the parse-lite tuples rather than full
/// [`LogOp`]s — the full op is read back lazily on demand via the
/// offset index (see [`JsonlStorage::read_op_at`]).
pub(super) enum RecordRead {
    /// Clean end of file.
    Eof,
    /// A parsed record (one op's `(ts, node)`, or several recovered from
    /// a glued line) and the number of bytes it spanned on disk.
    Ops {
        consumed: u64,
        ops: Vec<(Hlc, Option<NodeId>)>,
    },
    /// A byte span carrying no usable op — a blank line, non-UTF8 bytes (a
    /// partial sync can leave them mid-file), or JSON that didn't parse. The
    /// span length is still reported so the caller keeps a correct offset and
    /// reads the *next* record instead of aborting the whole file. `reason` is
    /// `Some` for a real defect (worth a warning), `None` for a benign blank.
    Skip {
        consumed: u64,
        reason: Option<String>,
    },
}

/// Read one newline-delimited record, never failing on encoding.
///
/// Unlike [`BufRead::read_line`], this reads raw bytes via `read_until`, so a
/// non-UTF8 byte in the middle of a file does not abort the read — it is
/// surfaced as a skippable record. Only a genuine device I/O error returns
/// `Err`. This is what lets one corrupt line (torn tail, partial sync) cost a
/// single record instead of every op after it in the file.
pub(super) fn read_log_record<R: std::io::BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<RecordRead> {
    buf.clear();
    let n = reader.read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(RecordRead::Eof);
    }
    let consumed = n as u64;
    let Ok(text) = std::str::from_utf8(buf) else {
        return Ok(RecordRead::Skip {
            consumed,
            reason: Some("non-UTF8 bytes".to_string()),
        });
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(RecordRead::Skip {
            consumed,
            reason: None,
        });
    }
    match parse_log_line_lite(trimmed) {
        Ok(ops) => Ok(RecordRead::Ops { consumed, ops }),
        Err(e) => Ok(RecordRead::Skip {
            consumed,
            reason: Some(e.to_string()),
        }),
    }
}

/// Extract the `NodeId` an op targets, if any. Every `Op` variant
/// carries one — there is no op that touches zero nodes. Returns
/// `Option` so callers can `filter_map` cleanly.
pub(super) fn op_node(op: &Op) -> Option<NodeId> {
    match op {
        Op::Create { node, .. }
        | Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::SetCollapsed { node, .. }
        | Op::SnoozeRemind { node, .. } => Some(*node),
    }
}

/// Parse `<actor>` out of a filename like `ops-<actor>.jsonl`. Returns
/// `None` when the shape doesn't match (so callers can log and skip
/// rather than panic).
pub(super) fn parse_actor_from_ops_filename(name: &str) -> Option<ActorId> {
    let stem = name
        .strip_prefix("ops-")
        .and_then(|s| s.strip_suffix(".jsonl"))?;
    ulid::Ulid::from_string(stem).ok().map(ActorId)
}

impl Storage for JsonlStorage {
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError> {
        self.append_op_inner(op)
    }

    fn append_ops(&mut self, ops: &[LogOp]) -> Result<(), StorageError> {
        self.append_ops_inner(ops)
    }

    fn disallow_writes(&mut self) {
        self.writes_allowed = false;
    }

    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError> {
        // The offset index is the complete op set; the LRU is only a warm
        // accelerator (empty right after boot). Drive the result off the
        // index so an evicted op is never dropped, preferring a cache hit
        // over a disk seek per op.
        let mut out = Vec::new();
        for (actor, hlc) in self.index.ts_after(ts) {
            out.push(self.read_op_or_missing(actor, hlc)?);
        }
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }
    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        // MUST be the COMPLETE op set for `id`. After an empty-cache boot
        // the LRU can hold a proper SUBSET of a node's ops (e.g. a
        // `SetCollapsed`/`SetProp` appended this session) while the node's
        // historical `Edit` ops sit on disk. A warm-only answer would then
        // drop that Edit history and corrupt the block's text on Doc
        // rebuild (#129). Always drive off the per-node index, which is
        // complete; `cold_ops_for_node` prefers a warm LRU hit per op.
        //
        // An op the index lists but disk won't return is an `Err`, not a
        // shorter set: a silently-short answer here rebuilds the block's
        // `Doc` from partial `Edit` history and hands the user text they
        // never wrote. `Workspace` degrades on the error (keeps the text it
        // has, warns) rather than failing the boot.
        self.cold_ops_for_node(id)
    }

    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError> {
        // Index-driven: read every op this actor authored, from the cache
        // when warm and disk otherwise. Returns the complete set even
        // after the LRU has shed the actor's older ops.
        let mut out = Vec::new();
        for ts in self.index.ts_for_actor(id) {
            out.push(self.read_op_or_missing(id, ts)?);
        }
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
        // The offset index keys ARE the per-actor high-water marks, so no
        // op reads are needed. The index is complete (the cache is only a
        // subset), so this is correct even with an empty LRU after boot.
        Ok(self.index.last_ts_per_actor())
    }

    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError> {
        // Fast path: the cache holds the complete op set (unbounded and
        // warm). `cache ⊆ index` always, so equal counts mean equal sets.
        {
            let cache = self.cache.read();
            if cache.len() == self.index.total_len() {
                let mut out: Vec<LogOp> = cache.iter().map(|(_, op)| op.clone()).collect();
                drop(cache);
                out.sort_by_key(|op| op.ts);
                return Ok(out);
            }
        }
        // Cold path (full-replay boot): read each `.jsonl` SEQUENTIALLY,
        // one open per file, instead of an index-driven `File::open` +
        // seek PER OP. On a 211k-op install-clean boot the seek-per-op
        // path is 211k `File::open` syscalls — far worse than a single
        // streaming pass over each file.
        self.read_all_ops_sequential()
    }

    fn ops_since_per_actor(
        &self,
        cutoff: &BTreeMap<ActorId, Hlc>,
    ) -> Result<Vec<LogOp>, StorageError> {
        // The snapshot-boot delta path. The default impl clones `all_ops`
        // and filters — O(whole log). Override to read only the per-actor
        // tail the index reports (all ops for an actor unseen by the
        // snapshot; the strictly-after-cutoff slice otherwise), a small,
        // recent set on a healthy boot. Preferring warm cache entries
        // keeps a same-process re-read cheap.
        let mut out = Vec::new();
        for (actor, ts) in self.index.ts_since_per_actor(cutoff) {
            // An op the index knows about but that we cannot read back
            // is corruption, and dropping it here is the worst possible
            // response: this is the snapshot-boot delta, and the cutoff
            // of the NEXT snapshot comes from the index — so the new
            // snapshot would claim to include an op its body never saw,
            // and no later boot would ever replay it again. Fail instead;
            // `try_boot_from_snapshot` degrades to a full replay, which
            // reads the file sequentially and can still recover it.
            out.push(self.read_op_or_missing(actor, ts)?);
        }
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    fn resize_cache(&mut self, cap: usize) {
        // Delegate to the inherent method so test code can call the
        // same logic without going through the trait.
        JsonlStorage::resize_cache(self, cap);
    }
}
