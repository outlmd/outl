//! Per-actor secondary index: `NodeId → Vec<(Hlc, offset)>`.
//!
//! The HLC index in [`super::index::OffsetIndex`] answers "where does
//! this op live?" by timestamp. This one answers "which ops ever
//! touched this block?" — the query `ops_for_node` needs.
//!
//! Without it, `ops_for_node` would scan every op in the file. With
//! it (plus the mmapped file), a cold rebuild of a heavily-edited Yrs
//! `Doc` is O(ops-on-that-node), not O(total).
//!
//! Same on-disk shape as `ops-<actor>.idx`: JSONL, one entry per line,
//! append-only. Pure cache; rebuilt from the `.jsonl` when missing.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::LogOp;
use crate::storage::sidecar;
use crate::storage::StorageError;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct NodeEntry {
    node: NodeId,
    ts: Hlc,
    offset: u64,
}

impl NodeIndex {
    /// Bytes one persisted entry occupies. Exposed so a test can count
    /// records without restating the layout.
    #[cfg(test)]
    pub(crate) const RECORD_WIDTH: usize = <NodeEntry as sidecar::Record>::WIDTH;
}

impl sidecar::Record for NodeEntry {
    const WIDTH: usize = 16 + sidecar::wire::HLC + 8;

    fn offset(&self) -> u64 {
        self.offset
    }

    fn encode(&self, out: &mut Vec<u8>) {
        sidecar::wire::put_u128(out, self.node.0 .0);
        sidecar::wire::put_hlc(out, &self.ts);
        sidecar::wire::put_u64(out, self.offset);
    }

    fn decode(b: &[u8]) -> Self {
        const TS: usize = 16 + sidecar::wire::HLC;
        Self {
            node: NodeId(ulid::Ulid(sidecar::wire::get_u128(&b[..16]))),
            ts: sidecar::wire::get_hlc(&b[16..TS]),
            offset: sidecar::wire::get_u64(&b[TS..]),
        }
    }
}

/// Secondary index mapping each `NodeId` to every `(Hlc, offset)` that
/// ever touched it. Powers `JsonlStorage::ops_for_node` without
/// scanning the whole `.jsonl`.
#[derive(Default, Debug)]
pub struct NodeIndex {
    entries: HashMap<NodeId, Vec<(Hlc, u64)>>,
}

impl NodeIndex {
    /// Build an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the op at `(ts, offset)` touched `node`. Order
    /// matters: callers should insert in HLC order so the per-node
    /// vector stays sorted (recovery code relies on this).
    pub fn insert(&mut self, node: NodeId, ts: Hlc, offset: u64) {
        self.entries.entry(node).or_default().push((ts, offset));
    }

    /// Offsets of every op that touched `node`, in insertion order.
    pub fn get(&self, node: &NodeId) -> &[(Hlc, u64)] {
        self.entries
            .get(node)
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    /// Number of nodes tracked.
    pub fn node_count(&self) -> usize {
        self.entries.len()
    }

    /// Total entries across every node.
    pub fn total_len(&self) -> usize {
        self.entries.values().map(|v| v.len()).sum()
    }

    /// The largest byte offset recorded across every node, or `None` when
    /// empty. Mirrors [`super::index::OffsetIndex::max_offset`]: boot uses
    /// it to confirm the node index reaches the same extent of the `.jsonl`
    /// as the offset index. Every op targets a node, so a consistent pair
    /// shares the same max offset; a lag means a crash landed between the
    /// two sidecar appends and the node index must be rebuilt.
    pub fn max_offset(&self) -> Option<u64> {
        self.entries
            .values()
            .flat_map(|v| v.iter().map(|(_, o)| *o))
            .max()
    }

    /// Load from a `ops-<actor>.nodes.idx` sidecar. Returns:
    /// - `Ok(Some)` when the file exists and parses.
    /// - `Ok(None)` when missing, empty, or corrupt (caller rebuilds).
    /// - `Err` only on real I/O failure.
    pub fn load(path: &Path) -> Result<Option<Self>, StorageError> {
        let mut index = Self::new();
        let recovered = sidecar::load_entries::<NodeEntry, _>(path, "node index", |entry| {
            index.insert(entry.node, entry.ts, entry.offset);
        })?;
        let Some(recovered) = recovered else {
            return Ok(None);
        };
        debug!(
            "node index loaded from {} ({recovered} entries across {} nodes)",
            path.display(),
            index.node_count(),
        );
        Ok(Some(index))
    }

    /// Persist the full index to `path` atomically.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        // **Sorted by offset**, which `load_entries` requires and uses as
        // its integrity check: a fixed-width record has no parse to fail,
        // so out-of-order offsets are what tells a reader the file is
        // damaged. Grouping by node (what this used to emit) produced a
        // file every reader would have had to reject.
        let mut flat: Vec<NodeEntry> = self
            .entries
            .iter()
            .flat_map(|(node, entries)| {
                entries.iter().map(|(ts, offset)| NodeEntry {
                    node: *node,
                    ts: *ts,
                    offset: *offset,
                })
            })
            .collect();
        flat.sort_by_key(|e| e.offset);
        sidecar::save_entries(path, flat.into_iter())
    }

    /// Append one entry to `path` (hot path from `append_op`).
    pub fn append_to(path: &Path, node: NodeId, ts: Hlc, offset: u64) -> Result<(), StorageError> {
        sidecar::append_entry(path, &NodeEntry { node, ts, offset })
    }

    /// Rebuild by streaming the `.jsonl` once and recording every op
    /// that touches a node (every `Op` variant does — see
    /// `op_touches_node` in `super::jsonl`).
    pub fn rebuild_from_jsonl(
        jsonl_path: &Path,
        op_touches_node: impl Fn(&crate::op::Op) -> Option<NodeId>,
    ) -> Result<Self, StorageError> {
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
                let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<LogOp>();
                for op in stream.flatten() {
                    if let Some(node) = op_touches_node(&op.op) {
                        index.insert(node, op.ts, start);
                    }
                }
            }
            offset += n as u64;
        }
        Ok(index)
    }
}

/// Lock-protected multi-actor secondary index, keyed by actor id.
#[derive(Default)]
pub struct ActorNodeIndex {
    inner: RwLock<HashMap<ActorId, NodeIndex>>,
}

impl ActorNodeIndex {
    /// Build an empty multi-actor index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a fresh `(node, ts, offset)` triple for `actor`.
    pub fn insert(&self, actor: ActorId, node: NodeId, ts: Hlc, offset: u64) {
        self.inner
            .write()
            .entry(actor)
            .or_default()
            .insert(node, ts, offset);
    }

    /// Replace the entire index for one actor (used by `reload()`).
    pub fn replace(&self, actor: ActorId, index: NodeIndex) {
        self.inner.write().insert(actor, index);
    }

    /// Snapshot of the offsets of every op that touched `node` under
    /// `actor`, in insertion order.
    pub fn get(&self, actor: ActorId, node: NodeId) -> Vec<(Hlc, u64)> {
        self.inner
            .read()
            .get(&actor)
            .map(|i| i.get(&node).to_vec())
            .unwrap_or_default()
    }

    /// Snapshot of every actor known to the index. Used by
    /// `cold_ops_for_node` to enumerate peer actors without depending
    /// on which ops happen to be warm in the LRU cache.
    pub fn actors(&self) -> Vec<ActorId> {
        self.inner.read().keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::op::Op;
    use tempfile::TempDir;

    fn logop(g: &HlcGenerator, node: NodeId) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }
    }

    fn touches(op: &crate::op::Op) -> Option<NodeId> {
        match op {
            Op::Create { node, .. }
            | Op::Move { node, .. }
            | Op::Edit { node, .. }
            | Op::SetProp { node, .. }
            | Op::SetCollapsed { node, .. }
            | Op::SnoozeRemind { node, .. } => Some(*node),
        }
    }

    #[test]
    fn node_index_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("ops-x.nodes.idx");
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let n1 = NodeId::new();
        let n2 = NodeId::new();

        let mut index = NodeIndex::new();
        index.insert(n1, g.next(), 0);
        index.insert(n1, g.next(), 100);
        index.insert(n2, g.next(), 200);
        index.save(&path).unwrap();

        let loaded = NodeIndex::load(&path).unwrap().expect("should load");
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.get(&n1).len(), 2);
        assert_eq!(loaded.get(&n2).len(), 1);
        assert_eq!(loaded.total_len(), 3);
    }

    #[test]
    fn missing_index_loads_as_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nope.nodes.idx");
        assert!(NodeIndex::load(&path).unwrap().is_none());
    }

    #[test]
    fn rebuild_from_jsonl_finds_all_node_ops() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let n1 = NodeId::new();
        let n2 = NodeId::new();
        let a = logop(&g, n1);
        let b = logop(&g, n2);
        let c = logop(&g, n1); // n1 again

        let jsonl = tmp.path().join(format!("ops-{actor}.jsonl"));
        let body = format!(
            "{}\n{}\n{}\n",
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            serde_json::to_string(&c).unwrap(),
        );
        std::fs::write(&jsonl, &body).unwrap();

        let index = NodeIndex::rebuild_from_jsonl(&jsonl, touches).unwrap();
        assert_eq!(index.node_count(), 2);
        assert_eq!(index.get(&n1).len(), 2);
        assert_eq!(index.get(&n2).len(), 1);
    }

    #[test]
    fn actor_node_index_routes_by_actor() {
        let ai = ActorNodeIndex::new();
        let a = ActorId::new();
        let b = ActorId::new();
        let ga = HlcGenerator::new(a);
        let gb = HlcGenerator::new(b);
        let n = NodeId::new();
        ai.insert(a, n, ga.next(), 10);
        ai.insert(b, n, gb.next(), 20);
        assert_eq!(ai.get(a, n).len(), 1);
        assert_eq!(ai.get(b, n).len(), 1);
        assert_eq!(ai.get(a, n)[0].1, 10);
        assert_eq!(ai.get(b, n)[0].1, 20);
    }
}
