//! Workspace summary/statistics accessors.
//!
//! These are deliberately cheap, index-driven counts and timestamps for
//! human-facing reports. Heavy projections belong in `outl-actions` or the
//! CLI, not here.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::workspace::{Workspace, WorkspaceError};

impl Workspace {
    /// Total number of ops persisted across all storage shards.
    ///
    /// This is the authoritative op-log count, including ops folded into a
    /// snapshot boot as well as the post-cutoff delta held in memory. Use this
    /// when a human-facing report needs the whole count.
    pub fn total_ops(&self) -> Result<usize, WorkspaceError> {
        Ok(self.router.op_count()?)
    }

    /// Wall-clock time of the most recently persisted op, if any.
    ///
    /// This reads the per-actor high-water marks from storage, so it is
    /// correct even after a snapshot boot where the in-memory log holds
    /// only the delta.
    pub fn last_op_time(&self) -> Result<Option<SystemTime>, WorkspaceError> {
        let mut max_ms = None::<u64>;
        for (_, hlc) in self.router.last_ts_per_actor()? {
            max_ms = Some(match max_ms {
                Some(ms) => ms.max(hlc.physical_ms),
                None => hlc.physical_ms,
            });
        }
        Ok(max_ms.map(|ms| UNIX_EPOCH + std::time::Duration::from_millis(ms)))
    }
}
