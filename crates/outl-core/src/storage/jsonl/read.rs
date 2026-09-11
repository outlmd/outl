//! Read / reload path for [`JsonlStorage`].
//!
//! Everything that pulls ops back off disk lives here: the boot-time
//! `reload` (global multi-actor merge and per-page single-file), the
//! cold-read fallbacks that rehydrate an evicted op via the offset
//! index, and the per-node cold walk. All of it is an inherent
//! `impl JsonlStorage` block; the child module sees the struct's
//! private fields.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tracing::{debug, error, warn};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::LogOp;
use crate::storage::sidecar::{self, SidecarKind};
use crate::storage::{NodeIndex, OffsetIndex, PageScope, StorageError};

use super::{
    parse_actor_from_ops_filename, parse_log_line, read_log_record, JsonlStorage, RecordRead,
};

/// Parse-lite ops recovered from one physical record: `(ts, node)` per op
/// (several when a glued line is recovered). Same shape as
/// [`RecordRead::Ops`]'s payload.
type LiteOps = Vec<(Hlc, Option<NodeId>)>;

/// How many *consecutive* unreadable records to tolerate before concluding
/// the file (or the device) is gone rather than hiccuping, and stopping.
///
/// Skipping is right for a one-off bad record; spinning forever on a dead
/// file handle is not. Shared by every sequential pass over a `.jsonl`
/// ([`JsonlStorage::read_ops_file_into`], [`JsonlStorage::rebuild_actor_indexes`])
/// so both bound the damage the same way.
const MAX_CONSECUTIVE_IO_ERRORS: usize = 64;

impl JsonlStorage {
    /// Read a single op from disk by `(actor, ts)`. Returns `None`
    /// when the HLC isn't in the offset index or the read fails.
    ///
    /// Cold path: only used when an op has been evicted from the LRU.
    /// Hot ops come straight out of `cache`. Uses seek + line read
    /// (no mmap) so the `unsafe`-free invariant of the crate holds.
    ///
    /// Under `PageScope::PerPage`, the offset index only knows about
    /// `self.actor`'s ops for this page (peer ops under PerPage come
    /// from their own `<peer-actor>/<slug>.jsonl` file). Cross-actor
    /// cold reads walk the per-actor file directory.
    fn read_op_at(&self, actor: ActorId, ts: Hlc) -> Option<LogOp> {
        let offset = self.index.get(actor, ts)?;
        let path = self.ops_path_for_actor(actor)?;
        let mut file = File::open(&path).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut line = String::new();
        BufReader::new(file).read_line(&mut line).ok()?;
        if line.is_empty() {
            return None;
        }
        // A glued line yields multiple ops; pick the one whose HLC
        // matches (same recovery semantics as the boot path).
        let ops = parse_log_line(line.trim()).ok()?;
        ops.into_iter().find(|o| o.ts == ts)
    }

    /// Read one op, preferring a warm LRU entry over a disk seek.
    ///
    /// Cache is keyed by the globally-unique HLC, so a hit is the exact
    /// op. On a miss (the common case right after boot, when the LRU is
    /// empty) it falls through to [`Self::read_op_at`]. Returns `None`
    /// when neither source can produce the op (e.g. a torn line the full
    /// parse rejects). Used by the cold-fallback `Storage` read methods.
    pub(super) fn read_op_hybrid(&self, actor: ActorId, ts: Hlc) -> Option<LogOp> {
        if let Some(op) = self.cache.read().peek(&ts).cloned() {
            return Some(op);
        }
        self.read_op_at(actor, ts)
    }

    /// [`Self::read_op_hybrid`], but a miss is an **error** rather than
    /// a silently dropped op.
    ///
    /// The offset index is the complete op set, so "the index says this
    /// op exists, the file won't give it to us" means the log is
    /// damaged. Every caller that assembles a *result set* — `ops_since`,
    /// `ops_for_actor`, `ops_since_per_actor`, `ops_for_node` — must use
    /// this: omitting the op yields a short read that looks exactly like a
    /// healthy short read, and downstream the snapshot cutoff is derived
    /// from the index — so the omission gets baked in as "already folded
    /// in" and the op is never replayed again.
    pub(super) fn read_op_or_missing(
        &self,
        actor: ActorId,
        ts: Hlc,
    ) -> Result<LogOp, StorageError> {
        self.read_op_hybrid(actor, ts).ok_or_else(|| {
            StorageError::MissingOp(format!(
                "op {ts:?} by actor {actor} is in the offset index but could not be read back \
                 from disk"
            ))
        })
    }

    /// Path of the `.jsonl` for `actor` under this storage's scope.
    /// Under Global that's `ops/ops-<actor>.jsonl`; under PerPage it's
    /// `ops/<actor>/<slug>.jsonl` (and only `self.actor` is expected).
    fn ops_path_for_actor(&self, actor: ActorId) -> Option<PathBuf> {
        match &self.scope {
            PageScope::Global => Some(self.ops_dir.join(format!("ops-{actor}.jsonl"))),
            PageScope::PerPage(slug) => {
                if actor == self.actor {
                    Some(
                        self.ops_dir
                            .join(self.actor.to_string())
                            .join(format!("{slug}.jsonl")),
                    )
                } else {
                    // PerPage doesn't currently merge peer files; the
                    // caller (cold_ops_for_node) iterates this actor
                    // only. Returning None makes the cold read a no-op
                    // for peers, which is correct under Phase B's
                    // single-page-per-storage model.
                    None
                }
            }
        }
    }

    /// Re-read every `ops-*.jsonl` from disk into the cache.
    ///
    /// Under `PageScope::Global` this scans every `ops-<actor>.jsonl`
    /// in `ops_dir` (legacy multi-actor merge). Under
    /// `PageScope::PerPage(slug)` it only opens the single file for
    /// `(this actor, this slug)` — that's the whole point of Phase B:
    /// boot is proportional to one page, not the whole workspace.
    pub fn reload(&mut self) -> Result<(), StorageError> {
        let scope = self.scope.clone();
        match scope {
            PageScope::Global => self.reload_global(),
            PageScope::PerPage(slug) => self.reload_per_page(&slug),
        }
    }

    fn reload_global(&mut self) -> Result<(), StorageError> {
        let mut per_file: Vec<(String, u64, usize, usize)> = Vec::new();
        let dir = std::fs::read_dir(&self.ops_dir)
            .map_err(|e| StorageError::Backend(format!("read {}: {e}", self.ops_dir.display())))?;

        // Reset every per-actor index — reload rebuilds from scratch.
        let mut seen_hlc: HashMap<ActorId, OffsetIndex> = HashMap::new();
        let mut seen_node: HashMap<ActorId, NodeIndex> = HashMap::new();

        for entry in dir {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("skipping unreadable entry: {e}");
                    continue;
                }
            };
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if !name.starts_with("ops-") || !name.ends_with(".jsonl") {
                continue;
            }
            let file_size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            // Readability guard: a file we can't open contributes nothing,
            // but must not fail the whole workspace open.
            if let Err(e) = File::open(&path) {
                warn!("cannot open {}: {e}", path.display());
                per_file.push((name, file_size, 0, 0));
                continue;
            }
            let file_actor = match parse_actor_from_ops_filename(&name) {
                Some(actor) => actor,
                None => {
                    // A file-sync tool's conflict copy — `ops-<id> 2.jsonl`
                    // (iCloud) or `ops-<id>.sync-conflict-*.jsonl` (Syncthing)
                    // — matches the `ops-*.jsonl` shape but carries no valid
                    // actor id. Skip it and keep going: one stray file next to
                    // the real logs must never fail the whole workspace open.
                    warn!("skipping ops file with unparseable actor id: {name}");
                    continue;
                }
            };

            // Load the persisted sidecars when they exactly cover the
            // `.jsonl`; rebuild via parse-lite otherwise. The `.idx` bytes are
            // the ~4.3s reload win — we skip re-tokenizing every `text_op`.
            let hlc_path = sidecar::path_for(
                &self.ops_dir,
                file_actor,
                &PageScope::Global,
                SidecarKind::Offset,
            );
            let node_path = sidecar::path_for(
                &self.ops_dir,
                file_actor,
                &PageScope::Global,
                SidecarKind::Node,
            );
            let (hlc_idx, node_idx) = Self::load_actor_indexes(&path, &hlc_path, &node_path);
            let ops_indexed = hlc_idx.len();
            seen_hlc.insert(file_actor, hlc_idx);
            seen_node.insert(file_actor, node_idx);
            debug!(
                "jsonl file {} size={} ops_indexed={}",
                name, file_size, ops_indexed
            );
            per_file.push((name, file_size, ops_indexed, ops_indexed));
        }

        let total_ops: usize = per_file.iter().map(|(_, _, _, ops)| ops).sum();
        debug!(
            "jsonl storage (global) indexed {} ops from {} ({} files)",
            total_ops,
            self.ops_dir.display(),
            per_file.len()
        );

        // Boot leaves the LRU empty. The offset/node indexes above are all
        // boot needs; the full ops (with their heavy `text_op` bytes) are
        // read back lazily from disk on demand via the offset index (see
        // `read_op_at` / the cold-fallback `Storage` methods). Filling the
        // cache here is what reparsed and re-allocated the whole log on
        // every open — RFC #137 Front A removes that.
        self.cache.write().clear();
        for (actor, idx) in seen_hlc {
            self.index.replace(actor, idx);
        }
        for (actor, idx) in seen_node {
            self.node_index.replace(actor, idx);
        }
        Ok(())
    }

    /// Reload under `PageScope::PerPage(slug)`. Opens exactly one file:
    /// `ops/<actor>/<slug>.jsonl`. Boot cost is O(this page's history),
    /// not O(workspace history).
    fn reload_per_page(&mut self, slug: &str) -> Result<(), StorageError> {
        let path = self.own_ops_path();
        let own_dir = self.own_ops_dir();
        let hlc_path = sidecar::path_for(&own_dir, self.actor, self.scope(), SidecarKind::Offset);
        let node_path = sidecar::path_for(&own_dir, self.actor, self.scope(), SidecarKind::Node);

        // Same load-or-rebuild path as `reload_global`, over the single
        // `<actor>/<slug>.jsonl` this storage owns. A missing file (fresh
        // page) yields empty indexes without writing spurious sidecars.
        let (hlc_idx, node_idx) = Self::load_actor_indexes(&path, &hlc_path, &node_path);

        debug!(
            "jsonl storage (per-page {}) indexed {} ops from {}",
            slug,
            hlc_idx.len(),
            path.display()
        );

        // Boot leaves the LRU empty — see `reload_global`. Cold ops come
        // back from disk on demand via the offset index.
        self.cache.write().clear();
        self.index.replace(self.actor, hlc_idx);
        self.node_index.replace(self.actor, node_idx);
        Ok(())
    }

    /// Build the offset + node index pair for one `.jsonl`, preferring the
    /// persisted sidecars over a full parse-lite rebuild.
    ///
    /// The `.idx` / `.nodes.idx` are a **cache**, never source of truth, so
    /// this is deliberately conservative: a wrong "fresh" verdict silently
    /// loses ops (this is the CRDT op log — there is no second chance), while
    /// a needless rebuild is only slower. When in doubt, rebuild.
    ///
    /// Decision ladder (see [`Self::try_fresh_actor_indexes`]):
    /// 1. Load both sidecars. Missing / corrupt / I/O error on either →
    ///    rebuild.
    /// 2. Validate the loaded pair exactly covers the `.jsonl`. Byte-exact
    ///    coverage → use as-is (no reparse). Appended-since (file grew) →
    ///    index only the TAIL and persist. Anything suspicious (truncated
    ///    file, index past EOF, tail record doesn't parse, tail ts/offset
    ///    disagree, node index lags) → rebuild.
    fn load_actor_indexes(
        jsonl_path: &Path,
        idx_path: &Path,
        node_idx_path: &Path,
    ) -> (OffsetIndex, NodeIndex) {
        match Self::try_fresh_actor_indexes(jsonl_path, idx_path, node_idx_path) {
            Some(pair) => pair,
            None => Self::rebuild_actor_indexes(jsonl_path, idx_path, node_idx_path),
        }
    }

    /// Attempt to satisfy the load from the persisted sidecars. Returns
    /// `Some((offset, node))` when the sidecars can be trusted — either
    /// byte-exact fresh, or grown-and-tail-reindexed — and `None` to signal
    /// the caller must rebuild from scratch. See [`Self::load_actor_indexes`]
    /// for the full rationale; every `None` branch is a conservative "the
    /// cache disagrees with the file, don't trust it".
    fn try_fresh_actor_indexes(
        jsonl_path: &Path,
        idx_path: &Path,
        node_idx_path: &Path,
    ) -> Option<(OffsetIndex, NodeIndex)> {
        // Both sidecars must load cleanly. A `None` (missing / corrupt) or a
        // real I/O `Err` on either side falls through to a full rebuild.
        let mut offset_index = OffsetIndex::load(idx_path).ok().flatten()?;
        let mut node_index = NodeIndex::load(node_idx_path).ok().flatten()?;

        let file_size = std::fs::metadata(jsonl_path).ok()?.len();

        // Extent the offset index claims to cover: the record at its MAX
        // offset. An empty index is only trustworthy over an empty file.
        let (off_max, entries_at_max) = match offset_index.max_offset_and_count() {
            Some(v) => v,
            None => {
                return if file_size == 0 {
                    Some((offset_index, node_index))
                } else {
                    None
                }
            }
        };

        // A non-empty index over an empty file is inconsistent.
        if file_size == 0 {
            return None;
        }

        // The node index must reach the same extent. Every op targets a node,
        // so a consistent pair shares the same max offset; a lag (a crash
        // between the two sidecar appends) means the node index is missing the
        // tail — rebuild rather than trust a partial one (`ops_for_node` feeds
        // block-text rebuild, #129).
        if node_index.max_offset() != Some(off_max) {
            return None;
        }

        // Read the physical record at `off_max`. It must parse, its byte
        // length gives the covered extent, and its ops must be exactly the
        // entries the index pins to `off_max`.
        let (consumed, ops_at_max) = Self::read_record_at(jsonl_path, off_max)?;
        if ops_at_max.len() != entries_at_max {
            return None;
        }
        for (ts, _) in &ops_at_max {
            if offset_index.get(ts) != Some(off_max) {
                return None;
            }
        }
        let covered_end = off_max + consumed;

        if covered_end == file_size {
            // Fresh: byte-exact coverage. Use as-is, no reparse — this is the
            // whole point of the optimization.
            return Some((offset_index, node_index));
        }
        if covered_end > file_size {
            // Index points past EOF (the `.jsonl` was truncated / shrank).
            return None;
        }

        // Grew: the log appended since the sidecars were last written. The op
        // log only ever appends, so the prefix `[0, covered_end)` is already
        // indexed identically — index only the TAIL `[covered_end, EOF)` and
        // merge it into the loaded pair. O(delta), not O(log).
        Self::reindex_tail(jsonl_path, covered_end, &mut offset_index, &mut node_index)?;
        // Persist the extended sidecars so the next boot is byte-exact fresh.
        if let Err(e) = offset_index.save(idx_path) {
            warn!("could not persist index {}: {e}", idx_path.display());
        }
        if let Err(e) = node_index.save(node_idx_path) {
            warn!(
                "could not persist node index {}: {e}",
                node_idx_path.display()
            );
        }
        Some((offset_index, node_index))
    }

    /// Read the single record starting at `offset` in the `.jsonl`, returning
    /// its byte length and parse-lite ops. `None` on any seek/read failure or
    /// when the bytes there don't parse as an op record — both of which mean
    /// the index disagrees with the file and the caller must rebuild.
    fn read_record_at(path: &Path, offset: u64) -> Option<(u64, LiteOps)> {
        let mut file = File::open(path).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut reader = BufReader::new(file);
        let mut buf: Vec<u8> = Vec::new();
        match read_log_record(&mut reader, &mut buf).ok()? {
            RecordRead::Ops { consumed, ops } => Some((consumed, ops)),
            // A blank/non-UTF8/unparseable record, or EOF, where the index
            // says an op lives is suspicious → rebuild.
            RecordRead::Skip { .. } | RecordRead::Eof => None,
        }
    }

    /// Index the tail of a `.jsonl` starting at `start_offset` into the given
    /// (already-loaded) indexes, using the same parse-lite pass and offset
    /// accounting as a full rebuild. `None` on a hard I/O error (caller
    /// rebuilds). Malformed lines are skipped exactly as the rebuild does.
    fn reindex_tail(
        path: &Path,
        start_offset: u64,
        offset_index: &mut OffsetIndex,
        node_index: &mut NodeIndex,
    ) -> Option<()> {
        let mut file = File::open(path).ok()?;
        file.seek(SeekFrom::Start(start_offset)).ok()?;
        let mut reader = BufReader::new(file);
        let mut buf: Vec<u8> = Vec::new();
        let mut offset = start_offset;
        loop {
            let start = offset;
            let record = read_log_record(&mut reader, &mut buf).ok()?;
            match record {
                RecordRead::Eof => break,
                RecordRead::Skip { consumed, .. } => offset += consumed,
                RecordRead::Ops { consumed, ops } => {
                    for (ts, node) in &ops {
                        offset_index.insert(*ts, start);
                        if let Some(node) = node {
                            node_index.insert(*node, *ts, start);
                        }
                    }
                    offset += consumed;
                }
            }
        }
        Some(())
    }

    /// Rebuild both indexes from scratch by streaming the `.jsonl` once with
    /// the parse-lite pass (extracts `(ts, node)` per op; never tokenizes
    /// `Op::Edit`'s `text_op`). Same cost as the legacy full load, so a
    /// missing/stale sidecar never makes boot slower than before this
    /// optimization. Persists both sidecars unless the file doesn't exist or
    /// the pass hit a read error (see below).
    ///
    /// **A read error skips the record, it does not stop the pass.** This is
    /// the sharpest edge of invariant #5's read side, because the index is
    /// what *defines* the known op set: stopping here produces an index that
    /// never learns about the ops past the damage, so
    /// [`Self::read_op_or_missing`] has nothing to complain about and the
    /// whole `StorageError::MissingOp` mechanism is bypassed — the tree boots
    /// short with no error anywhere. A skipped record costs its own bytes and
    /// nothing after them.
    ///
    /// A pass that hit a read error also **does not persist** the sidecars.
    /// I/O errors are transient in a way parse errors are not, so the index
    /// it produced is missing ops the next boot could well read fine. Writing
    /// it out would let the freshness check declare that short index
    /// byte-exact and make the omission permanent — the "wrong fresh verdict"
    /// this module's decision ladder exists to avoid.
    fn rebuild_actor_indexes(
        jsonl_path: &Path,
        idx_path: &Path,
        node_idx_path: &Path,
    ) -> (OffsetIndex, NodeIndex) {
        let mut hlc = OffsetIndex::new();
        let mut node = NodeIndex::new();

        let file = match File::open(jsonl_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // No file yet (fresh page / peer not synced). Empty indexes,
                // and DON'T write sidecars for a file that isn't there.
                return (hlc, node);
            }
            Err(e) => {
                warn!("cannot open {} to rebuild index: {e}", jsonl_path.display());
                return (hlc, node);
            }
        };

        let saw_io_error =
            Self::index_stream(&mut BufReader::new(file), jsonl_path, &mut hlc, &mut node);

        // Persist both sidecars next to the `.jsonl` — but only for a clean
        // pass. See this function's doc comment: caching an index built over
        // a transient read error is how a recoverable omission becomes a
        // permanent one.
        if saw_io_error {
            warn!(
                "not persisting indexes for {} — the rebuild hit read errors and the \
                 result is known-incomplete; the next boot rebuilds from the .jsonl",
                jsonl_path.display()
            );
            return (hlc, node);
        }

        if let Err(e) = hlc.save(idx_path) {
            warn!("could not persist index {}: {e}", idx_path.display());
        }
        if let Err(e) = node.save(node_idx_path) {
            warn!(
                "could not persist node index {}: {e}",
                node_idx_path.display()
            );
        }
        (hlc, node)
    }

    /// Drive the parse-lite indexing pass over one op-log stream, filling
    /// `hlc` and `node` with `(ts → offset)` / `(node, ts) → offset`.
    /// `label` names the source in log lines only. Returns whether the pass
    /// hit a read error, which is what tells the caller the index is
    /// known-incomplete and must not be cached.
    ///
    /// Split out of [`Self::rebuild_actor_indexes`] so the recovery
    /// behaviour can be driven by a reader that fails on demand — a real
    /// mid-file `read(2)` failure is not reproducible against a temp file,
    /// and this loop's `continue`-don't-`break` is precisely the part that
    /// must never regress.
    pub(super) fn index_stream<R: BufRead>(
        reader: &mut R,
        label: &Path,
        hlc: &mut OffsetIndex,
        node: &mut NodeIndex,
    ) -> bool {
        let mut offset: u64 = 0;
        let mut buf: Vec<u8> = Vec::new();
        let mut lines_read = 0usize;
        let mut io_errors = 0usize;
        let mut saw_io_error = false;
        loop {
            let start = offset;
            let record = match read_log_record(reader, &mut buf) {
                Ok(r) => r,
                Err(e) => {
                    lines_read += 1;
                    io_errors += 1;
                    saw_io_error = true;
                    warn!("io error {}:{}: {e}", label.display(), lines_read);
                    if io_errors >= MAX_CONSECUTIVE_IO_ERRORS {
                        error!(
                            "giving up indexing {} after {io_errors} consecutive io errors — \
                             ops past byte {offset} are NOT in the offset index for this boot",
                            label.display()
                        );
                        break;
                    }
                    // `read_until` leaves whatever it managed to read in
                    // `buf`, so the reader advanced by exactly that much.
                    // Advancing the offset by the same amount keeps every
                    // record AFTER the damage indexed at its true byte
                    // position — a drifted offset would make `read_op_at`
                    // seek to the wrong line, where the `ts` check rejects
                    // it, so it surfaces as `MissingOp` instead of a wrong
                    // op. Zero bytes read means no progress; the
                    // consecutive-error cap is what bounds that.
                    offset += buf.len() as u64;
                    continue;
                }
            };
            io_errors = 0;
            match record {
                RecordRead::Eof => break,
                RecordRead::Skip { consumed, reason } => {
                    lines_read += 1;
                    if let Some(reason) = reason {
                        warn!("skipping {}:{}: {reason}", label.display(), lines_read);
                    }
                    offset += consumed;
                }
                RecordRead::Ops { consumed, ops } => {
                    lines_read += 1;
                    if ops.len() > 1 {
                        warn!(
                            "recovered {} glued ops on {}:{} (concatenated JSON with no \
                             newline — likely an interleaved concurrent append)",
                            ops.len(),
                            label.display(),
                            lines_read
                        );
                    }
                    for (ts, n) in &ops {
                        hlc.insert(*ts, start);
                        if let Some(n) = n {
                            node.insert(*n, *ts, start);
                        }
                    }
                    offset += consumed;
                }
            }
        }
        saw_io_error
    }

    /// Cold-path `all_ops`: read every `.jsonl` this storage is
    /// responsible for SEQUENTIALLY — one open per file, streaming lines
    /// in order — instead of one `File::open` + seek per op.
    ///
    /// This is the full-replay boot path (211k ops on a mobile
    /// install-clean). Seeking per op via the offset index turns that into
    /// 211k `File::open` syscalls; a sequential stream reads each file
    /// once. Uses the FULL [`parse_log_line`] (not the parse-lite reload
    /// pass) because these ops are applied/sent — they need `text_op`.
    ///
    /// Files walked mirror `reload_global` / `reload_per_page`: every
    /// `ops-<actor>.jsonl` under Global, the single `<actor>/<slug>.jsonl`
    /// under PerPage. Disk is complete (`append_op` writes the line before
    /// caching), so this returns the full op set.
    pub(super) fn read_all_ops_sequential(&self) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = Vec::new();
        match &self.scope {
            PageScope::Global => {
                let dir = std::fs::read_dir(&self.ops_dir).map_err(|e| {
                    StorageError::Backend(format!("read {}: {e}", self.ops_dir.display()))
                })?;
                for entry in dir {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            warn!("skipping unreadable entry: {e}");
                            continue;
                        }
                    };
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default();
                    if !name.starts_with("ops-") || !name.ends_with(".jsonl") {
                        continue;
                    }
                    if parse_actor_from_ops_filename(name).is_none() {
                        // Conflict copy (`ops-<id> 2.jsonl`, sync-conflict);
                        // reload skips these, so must we.
                        continue;
                    }
                    Self::read_ops_file_into(&path, &mut out);
                }
            }
            PageScope::PerPage(_) => {
                let path = self.own_ops_path();
                Self::read_ops_file_into(&path, &mut out);
            }
        }
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    /// Stream one `.jsonl` file's ops into `out`, tolerating the same
    /// malformations as the reload path (non-UTF8 spans, blank lines, torn
    /// tails). A missing file is not an error (a peer file can vanish
    /// mid-sync, a PerPage page may have no ops yet) — it contributes
    /// nothing.
    ///
    /// A file that *exists* but won't open is a different animal: an entire
    /// peer's history drops out of this replay. It is logged at `error!`
    /// rather than returned as `Err` on purpose — failing here fails the
    /// whole workspace open, and the omission is self-healing: `reload`'s
    /// readability guard skips the same unopenable file, so the actor is
    /// absent from the offset index too, so the snapshot cutoff (derived from
    /// that index) never claims to have folded its ops in. The next boot that
    /// can open the file replays all of them. That is the opposite of the
    /// index-driven collectors, where the index *does* know the op and
    /// dropping it would be baked into the next cutoff — those return
    /// [`StorageError::MissingOp`].
    fn read_ops_file_into(path: &std::path::Path, out: &mut Vec<LogOp>) {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                error!(
                    "cannot open {}: {e} — NONE of this actor's ops are in this replay",
                    path.display()
                );
                return;
            }
        };
        let mut reader = BufReader::new(file);

        let mut buf: Vec<u8> = Vec::new();
        let mut line = 0usize;
        let mut io_errors = 0usize;
        loop {
            buf.clear();
            let n = match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    // `continue`, NOT `break`. This is the full-replay
                    // path: bailing here discards every remaining op in
                    // this actor's file — a transient error on line
                    // 5_000 of 200_000 would silently truncate the tree
                    // to the first 5_000 ops, and boot would carry on as
                    // if that were the whole workspace. Skipping the one
                    // unreadable line matches how a corrupt (non-UTF8 /
                    // unparseable) line is already handled below.
                    //
                    // A read error usually repeats, so bound the damage
                    // rather than spinning: give up after a run of them.
                    io_errors += 1;
                    warn!("io error {}:{}: {e}", path.display(), line + 1);
                    if io_errors >= MAX_CONSECUTIVE_IO_ERRORS {
                        error!(
                            "giving up on {} after {io_errors} consecutive io errors — \
                             the remainder of this actor's log was NOT replayed",
                            path.display()
                        );
                        break;
                    }
                    line += 1;
                    continue;
                }
            };
            io_errors = 0;
            line += 1;
            let _ = n;
            let Ok(text) = std::str::from_utf8(&buf) else {
                warn!("skipping {}:{}: non-UTF8 bytes", path.display(), line);
                continue;
            };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            match parse_log_line(trimmed) {
                Ok(ops) => out.extend(ops),
                Err(e) => warn!("skipping {}:{}: {e}", path.display(), line),
            }
        }
    }

    /// Cold-path `ops_for_node` when the LRU has no warm entry for the
    /// node. Walks the per-node secondary index across every known
    /// actor and pulls each op from the cache (if still resident) or
    /// the disk file. RFC #137 Phase A.
    ///
    /// The fourth index-driven collector, and it uses
    /// [`Self::read_op_or_missing`] for the same reason the other three do —
    /// with a sharper consequence. Its result rebuilds a block's Yrs `Doc`
    /// by replaying that node's `Edit` ops (#129), so an op quietly missing
    /// from the set does not shorten a list the caller can notice; it
    /// produces **wrong block text**, indistinguishable from text the user
    /// typed. An `Err` costs a stale-but-correct string (both call sites in
    /// `Workspace` warn and keep the text they already have); a short read
    /// costs the content.
    pub(super) fn cold_ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = Vec::new();
        // Derive the actor set from the node index (persistent), not
        // from the LRU cache (eviction-sensitive). If all ops for a
        // peer actor were evicted, the index still tracks them.
        for actor in self.node_index.actors() {
            let entries = self.node_index.get(actor, id);
            for (ts, _offset) in entries {
                // `read_op_or_missing` already prefers a warm LRU hit over
                // the disk seek, so there is no separate cache probe here.
                out.push(self.read_op_or_missing(actor, ts)?);
            }
        }
        out.sort_by_key(|op| op.ts);
        // HLCs are globally unique, so equal `ts` means the same op reached
        // us twice (e.g. a glued-line recovery that duplicated an index
        // entry). Dedup by HLC so the caller replays each op exactly once.
        out.dedup_by_key(|op| op.ts);
        Ok(out)
    }
}
