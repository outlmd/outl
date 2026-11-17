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
//! [`actor_census`] is the single owner of the answer, and it delegates to
//! `outl_core::storage::JsonlStorage::actor_timestamps`, which is the single
//! owner of reading it off disk.

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;

use crate::protocol::ActorClock;

/// Per-actor census of the local op log: the set of DISTINCT HLCs held for
/// each actor. Distinct because historic unsynchronized concurrent pulls left
/// duplicated append lines on real disks — a raw line count would inflate the
/// `count` and mask a genuine gap on the advertising side. The reference
/// workspace carries 148 such duplicates among 222,536 lines.
///
/// Read from the op log's offset index, never from the ops. See this module's
/// header for what that replaced and why.
pub(crate) fn actor_census(ops_dir: &std::path::Path) -> Result<HashMap<ActorId, BTreeSet<Hlc>>> {
    JsonlStorage::actor_timestamps(ops_dir).context("read the actor census from the op-log index")
}

/// Compute our vector clock (per-actor max HLC + distinct-op count) from the
/// local op log. Max + count both fall out of the same census pass.
pub(crate) fn local_vector_clock(
    ops_dir: &std::path::Path,
) -> Result<HashMap<ActorId, ActorClock>> {
    Ok(actor_census(ops_dir)?
        .into_iter()
        .filter_map(|(actor_id, hlcs)| {
            let max = *hlcs.last()?;
            Some((
                actor_id,
                ActorClock {
                    max,
                    count: hlcs.len() as u64,
                },
            ))
        })
        .collect())
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
