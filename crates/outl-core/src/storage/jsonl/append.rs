//! Write path for [`JsonlStorage`].
//!
//! The append that lands one-or-more ops on this actor's `.jsonl`,
//! including the torn-tail self-heal and the offset/node index mirroring.
//! An inherent `impl JsonlStorage` block; `Storage::append_op` /
//! `Storage::append_ops` in `mod.rs` forward straight to
//! [`JsonlStorage::append_op_inner`] / [`JsonlStorage::append_ops_inner`].
//!
//! A single append is just a batch of one: `append_op_inner` delegates to
//! `append_ops_inner` so the torn-tail heal and index-mirroring logic
//! lives once. What the batch buys is durability cost — the `.jsonl`
//! `fsync` (`F_FULLFSYNC` on macOS, ~4ms) happens ONCE for the whole lot
//! instead of once per op.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

use tracing::warn;

use crate::op::LogOp;
use crate::storage::sidecar::{self, SidecarKind};
use crate::storage::{NodeIndex, OffsetIndex, StorageError};

use super::{op_node, JsonlStorage};

impl JsonlStorage {
    pub(super) fn append_op_inner(&mut self, op: &LogOp) -> Result<(), StorageError> {
        self.append_ops_inner(std::slice::from_ref(op))
    }

    /// Append a batch of ops with a single `.jsonl` fsync for the lot.
    ///
    /// Rejects the whole batch (disk untouched) if any op belongs to a
    /// foreign actor, and treats an empty batch as a no-op. Otherwise it
    /// opens the file once, heals a torn tail once, writes every line
    /// tracking each one's byte offset, fsyncs once, then mirrors every op
    /// into both indexes and the LRU — same per-op index/cache semantics
    /// as the single-op path, just amortizing the durability barrier.
    pub(super) fn append_ops_inner(&mut self, ops: &[LogOp]) -> Result<(), StorageError> {
        // Validate EVERY op before touching the disk: a batch carrying a
        // foreign-actor op is rejected whole, file untouched.
        for op in ops {
            if op.actor != self.actor {
                return Err(StorageError::Backend(format!(
                    "refused to write op from foreign actor {} (we are {})",
                    op.actor, self.actor
                )));
            }
        }
        // Empty batch: nothing to write, no file to create, no fsync.
        if ops.is_empty() {
            return Ok(());
        }

        // Serialize every line up front so a serialization failure aborts
        // before we open or write anything.
        let lines: Vec<String> = ops
            .iter()
            .map(|op| serde_json::to_string(op).map_err(|e| StorageError::Serialize(e.to_string())))
            .collect::<Result<_, _>>()?;

        let path = self.own_ops_path();
        // Open the file once (read + append): the single handle serves the
        // offset probe, the torn-tail check, and the writes — instead of a
        // separate `metadata` stat plus a read-only open before the append
        // open. `O_APPEND` sends every write to EOF regardless of the read
        // cursor, so seeking back to read the last byte can't disturb where our
        // ops land. append is the SINGLE writer for its own actor file
        // (guarded by `ActorWriteLock`), so there is no concurrent-writer
        // TOCTOU between the tail check and the writes.
        let mut file = OpenOptions::new()
            .read(true)
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", path.display())))?;
        // Byte offset where the first line will land = current EOF. `O_APPEND`
        // leaves `stream_position()` at 0 until the first write, so seek to the
        // end explicitly to read it.
        let mut offset = file
            .seek(SeekFrom::End(0))
            .map_err(|e| StorageError::Backend(format!("seek {}: {e}", path.display())))?;
        // Torn-tail self-heal (ONCE for the whole batch). If the file doesn't
        // already end in a newline, a previous append was cut off mid-line
        // (crash / power loss / iOS jetsam), leaving a partial record with no
        // terminator. Appending straight after would glue our JSON onto that
        // fragment (`{"ts":…partial{"ts":…ours}`); the reader can't split a
        // torn prefix from a good op, so BOTH would be lost. Close the torn
        // line with a newline first, so the first batched op lands as its own
        // parseable record.
        let needs_separator = if offset > 0 {
            file.seek(SeekFrom::End(-1))
                .map_err(|e| StorageError::Backend(format!("seek {}: {e}", path.display())))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)
                .map_err(|e| StorageError::Backend(format!("read {}: {e}", path.display())))?;
            last[0] != b'\n'
        } else {
            false
        };
        if needs_separator {
            writeln!(file)
                .map_err(|e| StorageError::Backend(format!("heal tail {}: {e}", path.display())))?;
            // The first op's JSON now starts one byte later; the offset index
            // must point at the JSON, not the healing newline.
            offset += 1;
        }

        // Write every line, recording the byte offset each one lands at so the
        // indexes point at the JSON, not past it. `writeln!` adds one `\n`, so
        // each record spans `line.len() + 1` bytes.
        let mut offsets = Vec::with_capacity(lines.len());
        for line in &lines {
            offsets.push(offset);
            writeln!(file, "{line}")
                .map_err(|e| StorageError::Backend(format!("write {}: {e}", path.display())))?;
            offset += line.len() as u64 + 1;
        }
        // ONE fsync amortizes the durability barrier across the whole batch —
        // the single win this API exists for.
        file.sync_all()
            .map_err(|e| StorageError::Backend(format!("fsync {}: {e}", path.display())))?;

        // After durability: mirror every op into both indexes (in-memory +
        // sidecar append) and the LRU. Sidecar failures are best-effort — the
        // index is a cache, a missing entry just means the next boot rebuilds
        // from the .jsonl.
        let own_dir = self.own_ops_dir();
        let hlc_idx_path =
            sidecar::path_for(&own_dir, self.actor, self.scope(), SidecarKind::Offset);
        let node_idx_path =
            sidecar::path_for(&own_dir, self.actor, self.scope(), SidecarKind::Node);
        for (op, &offset) in ops.iter().zip(&offsets) {
            self.index.insert(op.actor, op.ts, offset);
            if let Err(e) = OffsetIndex::append_to(&hlc_idx_path, op.ts, offset) {
                warn!("could not append to index {}: {e}", hlc_idx_path.display());
            }
            if let Some(node) = op_node(&op.op) {
                self.node_index.insert(op.actor, node, op.ts, offset);
                if let Err(e) = NodeIndex::append_to(&node_idx_path, node, op.ts, offset) {
                    warn!(
                        "could not append to node index {}: {e}",
                        node_idx_path.display()
                    );
                }
            }
            self.cache.write().put(op.ts, op.clone());
        }
        Ok(())
    }
}
