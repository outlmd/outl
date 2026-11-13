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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Bound;

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::LogOp;
use tracing::info;

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

/// Most ops one exchange may carry, when the selection is one a budget may
/// safely cut.
///
/// A first sync used to cross as ONE frame: 719,785 ops / 253 MB against a
/// 256 MB frame cap, so it passed the guard by 1.2% and allocated instead of
/// failing. On an Android phone that is `Scudo ERROR: internal map failure`
/// across thirty threads and a `SIGABRT` on a `tokio-rt-worker`, every
/// launch, forever — the ingest lands on disk before the process dies, so
/// each retry re-pulls a slightly smaller mountain and dies again.
///
/// 20,000 ops is ~7 MB of JSONL on the reference workspace, which is a frame
/// a phone can hold while it is also opening a workspace. The whole history
/// still crosses; it crosses over roughly 36 exchanges at the catch-up
/// loop's cadence instead of one that cannot finish.
pub(crate) const MAX_OPS_PER_EXCHANGE: usize = 20_000;

/// Collect the ops the peer is missing, given the peer's vector clock,
/// bounded to [`MAX_OPS_PER_EXCHANGE`] where bounding is safe.
pub(crate) fn ops_missing_for(
    ops_dir: &std::path::Path,
    actor: ActorId,
    peer_clock: &HashMap<ActorId, ActorClock>,
) -> Result<Vec<LogOp>> {
    ops_missing_for_capped(ops_dir, actor, peer_clock, MAX_OPS_PER_EXCHANGE)
}

/// [`ops_missing_for`] with the budget spelled out, so a test can reach a
/// cut without writing twenty thousand ops.
///
/// Per actor in OUR log:
/// - peer has a GAP below its watermark (it holds fewer distinct ops `<=
///   max_r` than we do) → send that actor's FULL log, **never truncated**;
/// - peer has no entry → send everything we hold for that actor, budgeted;
/// - otherwise → send `ts > max_r`, budgeted.
///
/// **Why the gap resend is exempt.** A budget cuts the OLDEST ops, and a gap
/// resend exists precisely because we cannot tell which ops below the peer's
/// watermark are missing. Cut it, and the slice we send may be one the peer
/// already holds in full: its `(max, count)` comes back unchanged, the next
/// round computes the same cut, and the repair never progresses — silent
/// non-convergence, which is the one outcome this crate will not ship. An
/// oversized gap resend instead fails loudly on the frame cap, exactly as it
/// did before any budget existed. Pinned by
/// `a_gap_resend_is_never_truncated_by_the_budget`.
///
/// "Peer has no entry" looks identical to a gap resend today — both send
/// everything — and is **not** the same under a budget: a peer holding zero
/// ops of an actor cannot already have the slice we cut, so the oldest-first
/// prefix always lands. That difference is why the two are separate piles
/// here rather than one `full_resend` set.
///
/// Theoretical limitation (accepted, pre-dates the budget): two replicas
/// holding EQUAL counts of DIFFERENT subsets below the same max are
/// indistinguishable to this check, so the fast path under-sends between
/// them. Convergence still lands via the origin: each op's authoring device
/// holds its own actor's complete log, so any replica with a partial subset
/// trips the `below > count` check the next time it syncs with that origin.
pub(crate) fn ops_missing_for_capped(
    ops_dir: &std::path::Path,
    actor: ActorId,
    peer_clock: &HashMap<ActorId, ActorClock>,
    budget: usize,
) -> Result<Vec<LogOp>> {
    let census = actor_census(ops_dir)?;

    let gap_resend: HashSet<ActorId> = census
        .iter()
        .filter_map(|(actor_id, hlcs)| {
            let peer = peer_clock.get(actor_id)?;
            let below = hlcs.range(..=peer.max).count() as u64;
            (below > peer.count).then(|| {
                info!(
                    actor = %actor_id,
                    below,
                    peer_count = peer.count,
                    "gap below peer watermark detected; resending full actor log"
                );
                *actor_id
            })
        })
        .collect();

    let mut whole: Vec<(ActorId, Hlc)> = Vec::new();
    let mut budgeted: Vec<(ActorId, Hlc)> = Vec::new();
    for (actor_id, hlcs) in &census {
        if gap_resend.contains(actor_id) {
            whole.extend(hlcs.iter().map(|ts| (*actor_id, *ts)));
            continue;
        }
        let owed = match peer_clock.get(actor_id) {
            None => hlcs.iter().copied().collect::<Vec<Hlc>>(),
            Some(peer) => hlcs
                .range((Bound::Excluded(peer.max), Bound::Unbounded))
                .copied()
                .collect(),
        };
        budgeted.extend(owed.into_iter().map(|ts| (*actor_id, ts)));
    }

    // Oldest first, then cut. Sorting the whole selection by HLC leaves each
    // actor with a CONTIGUOUS prefix of what it owed, and that is the part
    // that matters: the receiver's `count` stays equal to what it holds below
    // its own `max`, so the next exchange reads as a fast path rather than
    // tripping the gap check this budget must never provoke.
    budgeted.sort_unstable_by_key(|(_, ts)| *ts);
    budgeted.truncate(budget);

    let mut picks = whole;
    picks.extend(budgeted);
    // Nothing to send? Then read nothing, and above all do not OPEN. The
    // census already answered the question off the offset index, and
    // "nothing" is the answer on a converged workspace — which is what the
    // catch-up loop asks every 8s per peer and what every pull-to-refresh
    // asks. Opening storage here is a full index load for an empty result.
    if picks.is_empty() {
        return Ok(Vec::new());
    }
    picks.sort_unstable_by_key(|(_, ts)| *ts);

    let storage =
        JsonlStorage::open(ops_dir.to_path_buf(), actor).context("open storage for delta")?;
    storage
        .read_ops_at(&picks)
        .context("read the ops this peer is missing")
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

    /// The catch-up loop asks "what does this peer still need" every 8s per
    /// peer, and on a converged workspace the answer is always "nothing".
    /// Reading the whole log to filter it down to empty is the standing cost
    /// of a quiet device, so the census answers first.
    #[test]
    fn a_peer_that_is_current_costs_no_op_read() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let actor = ActorId::new();
        write_log(&ops_dir, actor, 6);

        let theirs = local_vector_clock(&ops_dir).expect("clock");
        assert!(ops_missing_for(&ops_dir, actor, &theirs)
            .expect("delta")
            .is_empty());
        for path in sidecars(&ops_dir, actor) {
            assert!(
                !path.exists(),
                "{} means the ops were read; a peer holding our clock needs nothing",
                path.display()
            );
        }
    }

    /// The allow case, and it is the load-bearing one: an early-out that
    /// refuses everything passes every "sent nothing" assertion ever
    /// written.
    #[test]
    fn a_peer_behind_us_still_gets_every_op_it_lacks() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let (a, b) = (ActorId::new(), ActorId::new());
        let ops_a = write_log(&ops_dir, a, 6);
        let ops_b = write_log(&ops_dir, b, 3);

        // The peer holds the first two of A's ops and has never heard of B.
        let peer_clock: HashMap<ActorId, ActorClock> = [(
            a,
            ActorClock {
                max: ops_a[1].ts,
                count: 2,
            },
        )]
        .into_iter()
        .collect();

        let sent = ops_missing_for(&ops_dir, a, &peer_clock).expect("delta");
        let sent_ts: BTreeSet<Hlc> = sent.iter().map(|op| op.ts).collect();
        let expected: BTreeSet<Hlc> = ops_a[2..].iter().chain(&ops_b).map(|op| op.ts).collect();
        assert_eq!(sent_ts, expected);
    }

    /// A peer whose count is short of what we hold below its own watermark
    /// has a gap underneath it, so its actor's FULL log crosses — and the
    /// early-out must not swallow that, since nothing sits above the max.
    #[test]
    fn a_gap_below_the_peers_watermark_still_resends_the_full_log() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let actor = ActorId::new();
        let ops = write_log(&ops_dir, actor, 5);

        let peer_clock: HashMap<ActorId, ActorClock> = [(
            actor,
            ActorClock {
                max: ops.last().expect("op").ts,
                count: 2,
            },
        )]
        .into_iter()
        .collect();

        let sent = ops_missing_for(&ops_dir, actor, &peer_clock).expect("delta");
        assert_eq!(sent.len(), ops.len(), "the whole actor log must cross");
    }

    /// The vector clock a peer would report after holding exactly `held`.
    fn clock_of(held: &BTreeSet<Hlc>) -> ActorClock {
        ActorClock {
            max: *held.last().expect("a non-empty holding"),
            count: held.len() as u64,
        }
    }

    /// A first sync is the whole log, and the whole log does not fit in a
    /// phone. 719,785 ops crossed as ONE frame killed the Android app with
    /// `Scudo ERROR: internal map failure` and a `SIGABRT` on a
    /// `tokio-rt-worker`, every launch, forever — the op log is 253 MB
    /// against a 256 MB frame cap, so it squeaked under the guard and
    /// allocated instead of failing.
    ///
    /// The cap is on the SELECTION, not on the frame, because the sender
    /// blew up too: `ops_missing_for` read `all_ops` to filter it down.
    #[test]
    fn a_first_sync_is_capped_so_one_exchange_cannot_exhaust_memory() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let actor = ActorId::new();
        let ops = write_log(&ops_dir, actor, 50);

        let sent = ops_missing_for_capped(&ops_dir, actor, &HashMap::new(), 10).expect("delta");

        assert_eq!(sent.len(), 10, "one exchange must carry at most the budget");
        let sent_ts: Vec<Hlc> = sent.iter().map(|op| op.ts).collect();
        let oldest: Vec<Hlc> = ops[..10].iter().map(|op| op.ts).collect();
        assert_eq!(
            sent_ts, oldest,
            "the budget must take the OLDEST ops: a contiguous prefix per actor is what \
             keeps the receiver's (max, count) honest, so the next exchange reads as a \
             clean fast path instead of a gap"
        );
    }

    /// Capping is only safe if the remainder still crosses. Each exchange
    /// advances the receiver's clock, so the next one picks up where this
    /// one stopped — that is what makes a bounded frame converge instead of
    /// stalling a device halfway through its history.
    #[test]
    fn a_capped_first_sync_still_converges_over_repeated_exchanges() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let (a, b) = (ActorId::new(), ActorId::new());
        let ops_a = write_log(&ops_dir, a, 23);
        let ops_b = write_log(&ops_dir, b, 17);
        let everything: BTreeSet<Hlc> = ops_a.iter().chain(&ops_b).map(|op| op.ts).collect();

        let mut held: HashMap<ActorId, BTreeSet<Hlc>> = HashMap::new();
        for round in 0..100 {
            let peer_clock: HashMap<ActorId, ActorClock> = held
                .iter()
                .map(|(actor, hlcs)| (*actor, clock_of(hlcs)))
                .collect();
            let sent = ops_missing_for_capped(&ops_dir, a, &peer_clock, 7).expect("delta");
            if sent.is_empty() {
                let got: BTreeSet<Hlc> = held.values().flatten().copied().collect();
                assert_eq!(
                    got, everything,
                    "a round that sends nothing must mean convergence, not a stall \
                     (round {round})"
                );
                return;
            }
            assert!(sent.len() <= 7, "every round stays inside the budget");
            for op in sent {
                held.entry(op.ts.actor).or_default().insert(op.ts);
            }
        }
        panic!("the capped exchange never converged");
    }

    /// The one selection a budget must NOT touch.
    ///
    /// A gap resend exists because the peer is missing ops BELOW its own
    /// watermark, and we cannot tell which. Truncating it sends the oldest
    /// slice — which the peer may already hold in full, leaving its `(max,
    /// count)` unchanged and the next round identical. That is a silent
    /// non-convergence loop, and silence is the one failure this crate
    /// refuses. An oversized gap resend fails loudly at the frame cap
    /// instead, which is what it already did before the budget existed.
    #[test]
    fn a_gap_resend_is_never_truncated_by_the_budget() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        let actor = ActorId::new();
        let ops = write_log(&ops_dir, actor, 30);

        // Holds the newest op but only 2 ops in total: a gap underneath.
        let peer_clock: HashMap<ActorId, ActorClock> = [(
            actor,
            ActorClock {
                max: ops.last().expect("op").ts,
                count: 2,
            },
        )]
        .into_iter()
        .collect();

        let sent = ops_missing_for_capped(&ops_dir, actor, &peer_clock, 5).expect("delta");
        assert_eq!(
            sent.len(),
            ops.len(),
            "a gap resend crosses whole or the repair can make no progress"
        );
    }
}
