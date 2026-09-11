//! Per-actor offset index sidecar (`ops-<actor>.idx`).
//!
//! Maps each op's HLC to its byte offset inside the matching
//! `ops-<actor>.jsonl`. The index is what lets `JsonlStorage` read a
//! single cold op from a mmapped file without buffering the whole log
//! into RAM (RFC #137, Phase A — LRU + mmap).
//!
//! ## On-disk format
//!
//! JSONL, one entry per line, mirroring the `.jsonl` it indexes:
//!
//! ```jsonc
//! {"ts": {…Hlc…}, "offset": 1234}
//! ```
//!
//! JSONL (not a binary encoding) on purpose:
//!
//! - Same recovery path as the op log itself — `StreamDeserializer`
//!   catches glued writes.
//! - Append-only, so `append_op`'s index update is one `writeln!` +
//!   `fsync`, same cost shape as the op log append.
//! - Greppable and diffable, which matters the first time a real
//!   workspace hits an index bug.
//!
//! The index is a **cache**, not source of truth. Any time the `.idx`
//! is missing, truncated, or disagrees with the `.jsonl` line count,
//! we rebuild it from scratch by streaming the `.jsonl` once. That
//! rebuild is the same cost as the legacy full-load, so a missing
//! index never makes boot slower than today.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::hlc::Hlc;
use crate::op::LogOp;
use crate::storage::sidecar;
use crate::storage::StorageError;

/// One indexed entry: the HLC of an op + its byte offset inside the
/// `.jsonl` for that op's actor.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct IndexEntry {
    ts: Hlc,
    offset: u64,
}

/// In-memory offset index for a single actor's `.jsonl`.
///
/// Keyed by HLC (total order), so range queries ("every op after this
/// cutoff") are a `BTreeMap::range` away.
#[derive(Default, Debug)]
pub struct OffsetIndex {
    entries: BTreeMap<Hlc, u64>,
}

impl OffsetIndex {
    /// Build an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `ts` lives at `offset` in the underlying `.jsonl`.
    /// Inserting the same `(ts)` twice silently keeps the latest
    /// offset — the op log dedups by HLC on apply anyway.
    pub fn insert(&mut self, ts: Hlc, offset: u64) {
        self.entries.insert(ts, offset);
    }

    /// Look up the byte offset of the op identified by `ts`.
    pub fn get(&self, ts: &Hlc) -> Option<u64> {
        self.entries.get(ts).copied()
    }

    /// Offsets whose HLC sorts strictly after `cutoff`, in HLC order.
    pub fn after(&self, cutoff: Hlc) -> impl Iterator<Item = (&Hlc, &u64)> {
        self.entries.range((
            std::ops::Bound::Excluded(cutoff),
            std::ops::Bound::Unbounded,
        ))
    }

    /// Number of indexed ops.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The most recent HLC in this actor's index, or `None` when empty.
    /// `BTreeMap` keeps keys in HLC order, so this is the last key.
    pub fn last_ts(&self) -> Option<Hlc> {
        self.entries.keys().next_back().copied()
    }

    /// Every indexed HLC, in ascending HLC order.
    pub fn timestamps(&self) -> impl Iterator<Item = &Hlc> {
        self.entries.keys()
    }

    /// Whether the index holds zero ops.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The largest byte offset recorded, or `None` when empty.
    ///
    /// This is the on-disk position of the LAST physical record the index
    /// covers. Boot's freshness check seeks here to measure how far into
    /// the `.jsonl` the index actually reaches (`max_offset + line_len`).
    /// Scanning the values (not `.last()`) is robust even if append order
    /// and HLC order ever diverge.
    pub fn max_offset(&self) -> Option<u64> {
        self.entries.values().copied().max()
    }

    /// `(max_offset, how many entries map to it)`, or `None` when empty.
    ///
    /// The count lets the freshness check confirm the physical record at
    /// `max_offset` yields *exactly* the ops the index attributes to it —
    /// a glued line recovers several ops at one offset, so the record's op
    /// count must match the number of index entries pinned to that offset.
    pub fn max_offset_and_count(&self) -> Option<(u64, usize)> {
        let max = self.entries.values().copied().max()?;
        let count = self.entries.values().filter(|&&o| o == max).count();
        Some((max, count))
    }

    /// Load the index from `path`. Returns:
    ///
    /// - `Ok(Some(index))` when the file exists and parses cleanly.
    /// - `Ok(None)` when the file doesn't exist or is empty — caller
    ///   should rebuild.
    /// - `Err(_)` on a real I/O error.
    ///
    /// A truncated or otherwise malformed file logs a warning and
    /// returns `Ok(None)` so the caller can rebuild from the `.jsonl`.
    /// We never propagate parse failures as hard errors — the index is
    /// a cache.
    pub fn load(path: &Path) -> Result<Option<Self>, StorageError> {
        let mut index = Self::new();
        let recovered = sidecar::load_entries::<IndexEntry, _>(path, "index", |entry| {
            index.insert(entry.ts, entry.offset);
        })?;
        let Some(recovered) = recovered else {
            return Ok(None);
        };
        debug!("index loaded from {} ({recovered} entries)", path.display());
        Ok(Some(index))
    }

    /// Persist the full index to `path` atomically (tmp + rename).
    ///
    /// Used by `JsonlStorage` on shutdown and on explicit flush. The
    /// per-append hot path writes one line via [`Self::append_to`]
    /// instead, so this full save is for checkpointing only.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        sidecar::save_entries(
            path,
            self.entries.iter().map(|(ts, offset)| IndexEntry {
                ts: *ts,
                offset: *offset,
            }),
        )
    }

    /// Append a single entry to `path`. Cheap, durable, append-only —
    /// this is the hot path called from `JsonlStorage::append_op`.
    pub fn append_to(path: &Path, ts: Hlc, offset: u64) -> Result<(), StorageError> {
        sidecar::append_entry(path, &IndexEntry { ts, offset })
    }

    /// Rebuild the index by streaming the `.jsonl` once and recording
    /// the byte offset of each parsed op.
    ///
    /// Slow (O(jsonl size)) but correct. Used on first boot, after
    /// corruption, and by `reload()`. Same cost as the legacy full
    /// load, so a missing index never makes us slower than today.
    pub fn rebuild_from_jsonl(jsonl_path: &Path) -> Result<Self, StorageError> {
        let file = File::open(jsonl_path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", jsonl_path.display())))?;
        let mut reader = BufReader::new(file);
        let mut index = Self::new();
        let mut offset: u64 = 0;
        let mut buf = String::new();
        loop {
            let start = offset;
            buf.clear();
            let n = reader.read_line(&mut buf).map_err(|e| {
                StorageError::Backend(format!("read {}: {e}", jsonl_path.display()))
            })?;
            if n == 0 {
                break;
            }
            let trimmed = buf.trim();
            if !trimmed.is_empty() {
                // Reuse the same glued-op recovery the JsonlStorage
                // reload path uses. An entry per recovered op, all
                // pointing at the same line offset.
                let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<LogOp>();
                for op in stream.flatten() {
                    index.insert(op.ts, start);
                }
            }
            offset += n as u64;
        }
        Ok(index)
    }
}

/// Lock-protected multi-actor index, keyed by actor id.
///
/// `JsonlStorage` keeps one `OffsetIndex` per `ops-<actor>.jsonl`. This
/// wrapper is the cheap lookup: `index_for(actor)` borrows under a read
/// lock; mutation goes through a write lock.
#[derive(Default)]
pub struct ActorIndex {
    inner: RwLock<std::collections::HashMap<crate::id::ActorId, OffsetIndex>>,
}

impl ActorIndex {
    /// Build an empty multi-actor index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh `(ts, offset)` for `actor`.
    pub fn insert(&self, actor: crate::id::ActorId, ts: Hlc, offset: u64) {
        self.inner
            .write()
            .entry(actor)
            .or_default()
            .insert(ts, offset);
    }

    /// Replace the entire index for one actor (used by `reload()`).
    pub fn replace(&self, actor: crate::id::ActorId, index: OffsetIndex) {
        self.inner.write().insert(actor, index);
    }

    /// Snapshot of the offset for `(actor, ts)`, if known.
    pub fn get(&self, actor: crate::id::ActorId, ts: Hlc) -> Option<u64> {
        self.inner.read().get(&actor).and_then(|i| i.get(&ts))
    }

    /// Total entries across every actor.
    pub fn total_len(&self) -> usize {
        self.inner.read().values().map(|i| i.len()).sum()
    }

    /// The most recent HLC recorded per actor, straight from the offset
    /// index keys. This is the complete per-actor high-water mark — it
    /// does not depend on which ops are currently warm in the LRU, so a
    /// storage that boots with an empty cache still answers correctly.
    pub fn last_ts_per_actor(&self) -> std::collections::HashMap<crate::id::ActorId, Hlc> {
        self.inner
            .read()
            .iter()
            .filter_map(|(actor, idx)| idx.last_ts().map(|ts| (*actor, ts)))
            .collect()
    }

    /// Every `(actor, ts)` the index knows about, in no particular
    /// order. Callers rehydrate each op and sort by HLC afterwards.
    pub fn ts_iter(&self) -> Vec<(crate::id::ActorId, Hlc)> {
        let mut out = Vec::new();
        for (actor, idx) in self.inner.read().iter() {
            out.extend(idx.timestamps().map(|ts| (*actor, *ts)));
        }
        out
    }

    /// Every `(actor, ts)` whose HLC sorts strictly after `cutoff`,
    /// across every actor.
    pub fn ts_after(&self, cutoff: Hlc) -> Vec<(crate::id::ActorId, Hlc)> {
        let mut out = Vec::new();
        for (actor, idx) in self.inner.read().iter() {
            out.extend(idx.after(cutoff).map(|(ts, _)| (*actor, *ts)));
        }
        out
    }

    /// Every HLC recorded for `actor`, in ascending HLC order.
    pub fn ts_for_actor(&self, actor: crate::id::ActorId) -> Vec<Hlc> {
        self.inner
            .read()
            .get(&actor)
            .map(|idx| idx.timestamps().copied().collect())
            .unwrap_or_default()
    }

    /// The per-actor delta: for each actor, every HLC strictly after its
    /// mark in `cutoff`, or all of its HLCs when the actor is absent
    /// from `cutoff` (unseen — all its ops are still pending). Mirrors
    /// [`crate::storage::Storage::ops_since_per_actor`]'s semantics on
    /// the index so the `JsonlStorage` override never drops a lagging
    /// peer's low-HLC op below another actor's mark.
    pub fn ts_since_per_actor(
        &self,
        cutoff: &BTreeMap<crate::id::ActorId, Hlc>,
    ) -> Vec<(crate::id::ActorId, Hlc)> {
        let mut out = Vec::new();
        for (actor, idx) in self.inner.read().iter() {
            match cutoff.get(actor) {
                Some(c) => out.extend(idx.after(*c).map(|(ts, _)| (*actor, *ts))),
                None => out.extend(idx.timestamps().map(|ts| (*actor, *ts))),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::HlcGenerator;
    use crate::id::ActorId;
    use crate::op::Op;
    use tempfile::TempDir;

    fn logop(g: &HlcGenerator, op: Op) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    #[test]
    fn index_roundtrips_through_save_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("ops-test.idx");
        let mut index = OffsetIndex::new();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        for offset in [0u64, 128, 256, 1024] {
            let ts = g.next();
            index.insert(ts, offset);
        }
        index.save(&path).unwrap();
        let loaded = OffsetIndex::load(&path)
            .unwrap()
            .expect("index should load");
        assert_eq!(loaded.len(), 4);
        for ts in loaded.entries.keys() {
            assert_eq!(loaded.get(ts), index.get(ts));
        }
    }

    #[test]
    fn missing_index_loads_as_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.idx");
        assert!(OffsetIndex::load(&path).unwrap().is_none());
    }

    #[test]
    fn concurrent_saves_to_same_path_do_not_race_on_temp() {
        use std::sync::Arc;
        let tmp = TempDir::new().unwrap();
        let path = Arc::new(tmp.path().join("ops-race.idx"));
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut index = OffsetIndex::new();
        for offset in 0..64u64 {
            index.insert(g.next(), offset * 16);
        }
        let index = Arc::new(index);

        // Many threads checkpoint the SAME actor's index at once — the
        // reload/reindex race that produced `rename …idx.tmp -> …idx:
        // No such file or directory` when the temp name was shared. With
        // a per-write temp name every save must succeed.
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let index = Arc::clone(&index);
                let path = Arc::clone(&path);
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        index
                            .save(&path)
                            .expect("concurrent save must not race on the temp file");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        // The final index still loads cleanly after the storm.
        let loaded = OffsetIndex::load(&path).unwrap().expect("index loads");
        assert_eq!(loaded.len(), 64);
    }

    #[test]
    fn corrupt_index_rebuilds_as_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("corrupt.idx");
        std::fs::write(&path, b"not valid json at all").unwrap();
        assert!(OffsetIndex::load(&path).unwrap().is_none());
    }

    #[test]
    fn rebuild_from_jsonl_matches_offsets() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let jsonl = tmp.path().join(format!("ops-{actor}.jsonl"));

        let mk = || {
            logop(
                &g,
                Op::Create {
                    node: crate::id::NodeId::new(),
                    parent: crate::id::NodeId::root(),
                    position: crate::fractional::Fractional::first(),
                },
            )
        };
        let a = mk();
        let b = mk();
        let c = mk();

        // Write each op on its own line, tracking offsets.
        let line_a = serde_json::to_string(&a).unwrap();
        let line_b = serde_json::to_string(&b).unwrap();
        let line_c = serde_json::to_string(&c).unwrap();
        let body = format!("{line_a}\n{line_b}\n{line_c}\n");
        std::fs::write(&jsonl, &body).unwrap();

        let off_a = 0u64;
        let off_b = (line_a.len() + 1) as u64; // +1 for \n
        let off_c = off_b + (line_b.len() + 1) as u64;

        let index = OffsetIndex::rebuild_from_jsonl(&jsonl).unwrap();
        assert_eq!(index.len(), 3);
        assert_eq!(index.get(&a.ts), Some(off_a));
        assert_eq!(index.get(&b.ts), Some(off_b));
        assert_eq!(index.get(&c.ts), Some(off_c));
    }

    #[test]
    fn after_range_excludes_cutoff() {
        let mut index = OffsetIndex::new();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let t0 = g.next();
        let t1 = g.next();
        let t2 = g.next();
        index.insert(t0, 0);
        index.insert(t1, 10);
        index.insert(t2, 20);

        let after_t1: Vec<Hlc> = index.after(t1).map(|(ts, _)| *ts).collect();
        assert_eq!(after_t1, vec![t2]);
    }

    #[test]
    fn actor_index_routes_by_actor() {
        let ai = ActorIndex::new();
        let a = ActorId::new();
        let b = ActorId::new();
        let ga = HlcGenerator::new(a);
        let gb = HlcGenerator::new(b);
        let ta = ga.next();
        let tb = gb.next();

        ai.insert(a, ta, 100);
        ai.insert(b, tb, 200);

        assert_eq!(ai.get(a, ta), Some(100));
        assert_eq!(ai.get(b, tb), Some(200));
        assert_eq!(ai.get(a, tb), None); // wrong actor
        assert_eq!(ai.total_len(), 2);
    }
}
