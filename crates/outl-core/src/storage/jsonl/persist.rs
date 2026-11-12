//! Persisting a sidecar pair, and rebuilding one, under the [`IdxLock`].
//!
//! Split from `read.rs`: these are the only sidecar *writers* on the read
//! path, and the lock they take is what `idx_lock.rs` explains.

use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use tracing::warn;

use crate::id::ActorId;
use crate::storage::sidecar::{self, SidecarKind};
use crate::storage::{NodeIndex, OffsetIndex, PageScope, StorageError};

use super::idx_lock::IdxLock;
use super::{read_log_record, JsonlStorage, RecordRead};

impl JsonlStorage {
    /// Replace the sidecar pair on disk with `offset_index` / `node_index`,
    /// after making them cover the whole `.jsonl`.
    ///
    /// The live writer appends to these sidecars by path, one entry per op,
    /// under the same [`IdxLock`]. A persister that renamed a fresh copy over
    /// them without it would drop every entry appended since it read the
    /// file, and the max-offset check would still pass (see `idx_lock.rs`).
    /// So: take the lock, index whatever tail appeared since the in-memory
    /// pair was built, and only then rename. With the lock held no append is
    /// in flight, so the pair is complete at the moment it lands.
    ///
    /// Without the lock nothing is persisted: the sidecars are a cache and
    /// the next boot rebuilds, which is slower but cannot lose an op.
    pub(super) fn persist_indexes_locked(
        jsonl_path: &Path,
        idx_path: &Path,
        node_idx_path: &Path,
        offset_index: &mut OffsetIndex,
        node_index: &mut NodeIndex,
    ) {
        let _lock = match IdxLock::acquire(idx_path) {
            Ok(lock) => lock,
            Err(e) => {
                warn!("could not lock index {}: {e}; not persisting", idx_path.display());
                return;
            }
        };
        if Self::extend_to_eof(jsonl_path, offset_index, node_index).is_none() {
            warn!("not persisting indexes for {}: tail unreadable", jsonl_path.display());
            return;
        }
        if let Err(e) = offset_index.save(idx_path) {
            warn!("could not persist index {}: {e}", idx_path.display());
        }
        if let Err(e) = node_index.save(node_idx_path) {
            warn!(
                "could not persist node index {}: {e}",
                node_idx_path.display()
            );
        }
    }

    /// Index whatever the `.jsonl` holds past the record at the pair's max
    /// offset. Idempotent: a tail that is already indexed re-inserts the same
    /// entries. `None` on a hard I/O error, or when the max-offset record no
    /// longer parses (the file changed under us; the caller must not persist).
    pub(super) fn extend_to_eof(
        jsonl_path: &Path,
        offset_index: &mut OffsetIndex,
        node_index: &mut NodeIndex,
    ) -> Option<()> {
        let file_size = std::fs::metadata(jsonl_path).ok()?.len();
        let covered_end = match offset_index.max_offset() {
            None => 0,
            Some(off) => off + Self::read_record_at(jsonl_path, off)?.0,
        };
        if covered_end >= file_size {
            return Some(());
        }
        Self::reindex_tail(jsonl_path, covered_end, offset_index, Some(node_index))
    }

    /// Rebuild a device's OWN sidecar pair from its `.jsonl` and persist it,
    /// under the [`IdxLock`] the writer appends under.
    ///
    /// The self-heal for a sidecar that lost entries mid-file (the freshness
    /// ladder cannot see those). Streams the whole file, so call it only on
    /// evidence of a hole, never as routine. Returns the rebuilt offset
    /// index, whose keys are exactly the HLCs the `.jsonl` holds. Persists
    /// nothing (and errors) if the pass hit a read error.
    pub fn rebuild_own_indexes(
        ops_dir: &Path,
        actor: ActorId,
    ) -> Result<OffsetIndex, StorageError> {
        let jsonl_path = ops_dir.join(format!("ops-{actor}.jsonl"));
        let idx_path = sidecar::path_for(ops_dir, actor, &PageScope::Global, SidecarKind::Offset);
        let node_idx_path =
            sidecar::path_for(ops_dir, actor, &PageScope::Global, SidecarKind::Node);
        let _lock = IdxLock::acquire(&idx_path)
            .map_err(|e| StorageError::Backend(format!("lock {}: {e}", idx_path.display())))?;
        let file = File::open(&jsonl_path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", jsonl_path.display())))?;
        let mut hlc = OffsetIndex::new();
        let mut node = NodeIndex::new();
        if Self::index_stream(&mut BufReader::new(file), &jsonl_path, &mut hlc, Some(&mut node)) {
            return Err(StorageError::Backend(format!(
                "read error rebuilding {}; sidecars left as they were",
                jsonl_path.display()
            )));
        }
        hlc.save(&idx_path)?;
        node.save(&node_idx_path)?;
        Ok(hlc)
    }

    /// Index the tail of a `.jsonl` starting at `start_offset` into the given
    /// (already-loaded) indexes, using the same parse-lite pass and offset
    /// accounting as a full rebuild. `None` on a hard I/O error (caller
    /// rebuilds). Malformed lines are skipped exactly as the rebuild does.
    ///
    /// `node_index` is optional because the metadata-only read
    /// ([`Self::actor_timestamps`]) wants the HLCs and has no use for the
    /// node map.
    pub(super) fn reindex_tail(
        path: &Path,
        start_offset: u64,
        offset_index: &mut OffsetIndex,
        mut node_index: Option<&mut NodeIndex>,
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
                        if let (Some(node), Some(index)) = (node, node_index.as_deref_mut()) {
                            index.insert(*node, *ts, start);
                        }
                    }
                    offset += consumed;
                }
            }
        }
        Some(())
    }
}
