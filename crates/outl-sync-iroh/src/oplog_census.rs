//! What this device holds, and what a peer is therefore missing.
//!
//! The read half of the wire's op-log access. [`oplog`](crate::oplog) owns
//! the write half — the append serialization and the fsync-before-ack rule —
//! and those are durability invariants about bytes landing. This module owns
//! a different question: *which ops exist here*, answered without reading one.
//!
//! That distinction is the whole reason the module exists. A vector clock is
//! a max HLC and a distinct-op count per actor, and the op log's offset index
//! is keyed by HLC — so its keys **are** the answer. Deriving it from
//! `JsonlStorage::open` + `all_ops` instead cost a full workspace load per
//! question: both index sidecars for every actor, then every `Op::Edit`'s
//! `text_op` parsed back into a `Vec<u8>`. On the reference 222,536-op
//! workspace that is 52 MB of sidecar plus 80 MB of `.jsonl`, measured at
//! 342 ms per call in release and 3.7 s in debug.
//!
//! Three callers asked it on a loop. `peer_status` polls every 5s from both
//! GUI clients — on iOS from the IPC thread, which is the UI main thread, so
//! the scene-update watchdog killed the app before it could paint. The
//! catch-up loop asks it per peer every 8s. And the ingest asked it per
//! received batch *while holding the device-wide append flock*, so every
//! other sync write on the machine queued behind a workspace load.
//!
//! [`Census`] is the single owner of the answer, and it delegates to
//! `outl_core::storage::JsonlStorage::actor_offset_indexes`, which is the
//! single owner of reading it off disk.
//!
//! **Those three callers now ask once per exchange, not once per
//! question.** Reducing the constant was not enough: the read was
//! `O(total ops)` whatever it cost, and an exchange asked for it six times
//! (plus two `JsonlStorage::open`s) with the same answer every time. A
//! [`Census`] is read once and carries the offsets, so the clock, the
//! selection and the op bodies all come out of it. On the reference
//! 748,684-op workspace that is ~710 ms per exchange down to ~38 ms, and
//! the 160 MB of index the two opens allocated is not allocated at all.

use std::collections::HashMap;

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, OffsetIndex};

use crate::protocol::ActorClock;

/// Per-actor census of the local op log: for each actor, every DISTINCT HLC
/// this device holds **and the byte offset of the op behind it**.
///
/// Distinct because historic unsynchronized concurrent pulls left duplicated
/// append lines on real disks — a raw line count would inflate the `count`
/// and mask a genuine gap on the advertising side. The reference workspace
/// carries 148 such duplicates among 222,536 lines.
///
/// Read from the op log's offset index, never from the ops. See this
/// module's header for what that replaced and why.
///
/// **One read answers all three questions an exchange asks**, and that is
/// the point of carrying the offsets rather than just the HLCs:
///
/// - *what do we hold* → [`Self::vector_clock`];
/// - *what does this peer lack* → [`crate::oplog_select::ops_missing_for`],
///   which ranges over the same indexes;
/// - *where are those ops on disk* → the offsets, spent by
///   `JsonlStorage::read_ops_at_offsets`.
///
/// Each of those used to re-read the whole census from disk, and the third
/// additionally called `JsonlStorage::open` — a full load of both index
/// sidecars for every actor. Measured on a real 748,684-op / 23-actor
/// workspace: six censuses plus two opens per exchange, ~710 ms of CPU, to
/// move ops that are usually zero in number. The catch-up loop runs that
/// per peer every 8 s, so the device never left it.
pub(crate) struct Census {
    by_actor: HashMap<ActorId, OffsetIndex>,
}

impl Census {
    /// Read the census off the op log's offset index.
    pub(crate) fn read(ops_dir: &std::path::Path) -> Result<Self> {
        Ok(Self {
            by_actor: JsonlStorage::actor_offset_indexes(ops_dir)
                .context("read the actor census from the op-log index")?,
        })
    }

    /// Our vector clock: per-actor max HLC + distinct-op count. Both fall
    /// out of the index this census already holds.
    pub(crate) fn vector_clock(&self) -> HashMap<ActorId, ActorClock> {
        self.by_actor
            .iter()
            .filter_map(|(actor_id, index)| {
                Some((
                    *actor_id,
                    ActorClock {
                        max: index.last_ts()?,
                        count: index.len() as u64,
                    },
                ))
            })
            .collect()
    }

    /// Every actor this device holds ops for, with its index.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&ActorId, &OffsetIndex)> {
        self.by_actor.iter()
    }

    /// Every distinct HLC in the census, regardless of actor — the
    /// present-set the ingest dedups a received batch against.
    pub(crate) fn all_timestamps(&self) -> impl Iterator<Item = &Hlc> {
        self.by_actor.values().flat_map(|index| index.timestamps())
    }
}

/// Compute our vector clock from the local op log, for a caller that needs
/// nothing else from the census.
///
/// Prefer [`Census::read`] + [`Census::vector_clock`] wherever the same
/// exchange also asks what to send: that is one disk read instead of two.
pub(crate) fn local_vector_clock(
    ops_dir: &std::path::Path,
) -> Result<HashMap<ActorId, ActorClock>> {
    Ok(Census::read(ops_dir)?.vector_clock())
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::fractional::Fractional;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::NodeId;
    use outl_core::op::Op;
    use outl_core::storage::sidecar::{self, SidecarKind};
    use outl_core::storage::PageScope;
    use outl_core::LogOp;
    use std::path::{Path, PathBuf};

    /// Write one actor's `ops-<actor>.jsonl` by hand, returning its ops.
    fn write_log(ops_dir: &Path, actor: ActorId, count: usize) -> Vec<LogOp> {
        let gen = HlcGenerator::new(actor);
        let ops: Vec<LogOp> = (0..count)
            .map(|_| {
                let ts = gen.next();
                LogOp {
                    ts,
                    actor,
                    op: Op::Create {
                        node: NodeId::new(),
                        parent: NodeId::root(),
                        position: Fractional::first(),
                    },
                }
            })
            .collect();
        let body: String = ops
            .iter()
            .map(|op| format!("{}\n", serde_json::to_string(op).expect("encode")))
            .collect();
        std::fs::create_dir_all(ops_dir).expect("ops dir");
        std::fs::write(ops_dir.join(format!("ops-{actor}.jsonl")), body).expect("write");
        ops
    }

    /// The two index sidecars for `actor`. Their absence is what the tests
    /// below watch: a path that goes through `JsonlStorage::open` rebuilds
    /// and persists both, so "still missing" means "no full open happened".
    fn sidecars(ops_dir: &Path, actor: ActorId) -> [PathBuf; 2] {
        [SidecarKind::Offset, SidecarKind::Node]
            .map(|kind| sidecar::path_for(ops_dir, actor, &PageScope::Global, kind))
    }

    /// The clock is a max and a count per actor, and both come off the
    /// offset index. Rerouting it back through `JsonlStorage::open` —
    /// `reload` loading the node sidecar for every actor, then `all_ops`
    /// parsing every op body — is what put a full workspace load behind one
    /// `peer_status` call and got the iOS app killed by the watchdog.
    #[test]
    fn the_vector_clock_is_read_from_the_index_not_from_the_ops() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let (a, b) = (ActorId::new(), ActorId::new());
        let ops_a = write_log(&ops_dir, a, 11);
        write_log(&ops_dir, b, 4);

        let clock = local_vector_clock(&ops_dir).expect("clock");
        assert_eq!(clock[&a].count, 11);
        assert_eq!(clock[&a].max, ops_a.last().expect("op").ts);
        assert_eq!(clock[&b].count, 4);

        for path in sidecars(&ops_dir, a).iter().chain(&sidecars(&ops_dir, b)) {
            assert!(
                !path.exists(),
                "{} was built to answer a vector clock; the clock needs neither \
                 the node index nor an op body",
                path.display()
            );
        }
    }
}
