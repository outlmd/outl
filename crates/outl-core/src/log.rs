//! Append-only op log ordered by HLC.
//!
//! The log is the source of truth for the CRDT. `Tree::apply_op` calls
//! `append`/`pop` directly when reordering; outside code should treat the
//! log as read-only and route mutations through `Workspace`.

use std::collections::HashMap;

use crate::hlc::Hlc;
use crate::id::NodeId;
use crate::op::{LogOp, Op};

/// In-memory op log, sorted by HLC.
#[derive(Debug, Default, Clone)]
pub struct OpLog {
    ops: Vec<LogOp>,
    /// `node -> positions of that node's `Edit` ops in `ops`. Lets
    /// [`Self::edit_updates`] rebuild a block's text in O(edits-of-node),
    /// in memory, instead of scanning the whole log per block (O(log)) or
    /// hitting the on-disk per-node index (a cold seek per op). Kept in sync
    /// by `append`/`pop` — the only mutators. Positions stay valid because
    /// `Edit`s are appended at the tail and popped from the tail during
    /// reorder, so a node's position list is always tail-consistent.
    edits_by_node: HashMap<NodeId, Vec<usize>>,
}

impl OpLog {
    /// Build an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of ops currently in the log.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Most recent op (highest HLC), if any.
    pub fn last(&self) -> Option<&LogOp> {
        self.ops.last()
    }

    /// Append an op at the tail.
    ///
    /// Callers (`Tree::apply_op` in particular) are responsible for ensuring
    /// that `op.ts > self.last().ts`; the log does not re-sort on append.
    pub fn append(&mut self, op: LogOp) {
        if let Some(last) = self.ops.last() {
            debug_assert!(
                op.ts > last.ts,
                "OpLog::append called out of order: last.ts={:?} new.ts={:?}",
                last.ts,
                op.ts
            );
        }
        if let Op::Edit { node, .. } = &op.op {
            self.edits_by_node
                .entry(*node)
                .or_default()
                .push(self.ops.len());
        }
        self.ops.push(op);
    }

    /// Pop the most recent op. Used by `Tree::apply_op` while reordering.
    pub fn pop(&mut self) -> Option<LogOp> {
        let op = self.ops.pop()?;
        // The popped op was at the tail, so if it's an `Edit` its position is
        // the last entry in that node's list — drop it to keep the index in
        // sync with the shrunk `ops`.
        if let Op::Edit { node, .. } = &op.op {
            if let Some(positions) = self.edits_by_node.get_mut(node) {
                positions.pop();
            }
        }
        Some(op)
    }

    /// Iterate all ops in HLC order.
    pub fn iter(&self) -> impl Iterator<Item = &LogOp> {
        self.ops.iter()
    }

    /// The `Edit` update bytes for `node`, in HLC order, resolved through the
    /// per-node index — O(edits-of-node), in memory, no log scan and no disk.
    /// This is the hot path behind `Workspace::block_text`; scanning the whole
    /// resident log per block was O(log) each (pathological after a full
    /// replay), and the on-disk `ops_for_node` cold path did a seek per op.
    pub fn edit_updates(&self, node: NodeId) -> impl Iterator<Item = &[u8]> {
        self.edits_by_node
            .get(&node)
            .into_iter()
            .flatten()
            .filter_map(move |&i| match &self.ops[i].op {
                Op::Edit { text_op, .. } => Some(text_op.as_slice()),
                _ => None,
            })
    }

    /// Op at index `i` in HLC order, if any.
    ///
    /// Indices are only stable between reorders; callers that hold one
    /// across an `apply_op` must re-derive it.
    pub fn get(&self, i: usize) -> Option<&LogOp> {
        self.ops.get(i)
    }

    /// Whether the log already contains an op with this HLC.
    ///
    /// Implementation uses binary search over the HLC-sorted ops, so this
    /// is O(log n). `Tree::apply_op` calls this on every `apply` to enforce
    /// idempotency without quadratic cost.
    pub fn contains_ts(&self, ts: &Hlc) -> bool {
        self.get_by_ts(ts).is_some()
    }

    /// The op with this HLC, if the log holds it. O(log n), same binary
    /// search as [`Self::contains_ts`].
    ///
    /// Exists so a caller can read back the op **as the tree recorded
    /// it**. `Tree::do_op` fills the `old_*` fields on the copy that
    /// lands here, and those are what a reader of the log has to see —
    /// see `Workspace::apply`.
    pub fn get_by_ts(&self, ts: &Hlc) -> Option<&LogOp> {
        self.ops
            .binary_search_by(|o| o.ts.cmp(ts))
            .ok()
            .and_then(|i| self.ops.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{ActorId, NodeId};
    use crate::op::Op;

    fn op_at(physical: u64, logical: u32, actor: ActorId) -> LogOp {
        LogOp {
            ts: Hlc::new(physical, logical, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: crate::fractional::Fractional::first(),
            },
        }
    }

    #[test]
    fn append_and_last_track_tail() {
        let actor = ActorId::new();
        let mut log = OpLog::new();
        assert!(log.is_empty());
        log.append(op_at(1, 0, actor));
        log.append(op_at(2, 0, actor));
        assert_eq!(log.len(), 2);
        assert_eq!(log.last().unwrap().ts.physical_ms, 2);
    }

    #[test]
    fn pop_returns_most_recent() {
        let actor = ActorId::new();
        let mut log = OpLog::new();
        log.append(op_at(1, 0, actor));
        log.append(op_at(2, 0, actor));
        log.append(op_at(3, 0, actor));
        let popped = log.pop().unwrap();
        assert_eq!(popped.ts.physical_ms, 3);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn contains_ts_uses_total_order() {
        let actor = ActorId::new();
        let mut log = OpLog::new();
        let op = op_at(5, 0, actor);
        let ts = op.ts;
        log.append(op);
        assert!(log.contains_ts(&ts));
        let other = Hlc::new(5, 1, actor);
        assert!(!log.contains_ts(&other));
    }
}
