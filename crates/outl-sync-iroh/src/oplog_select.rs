//! What a peer is missing, and how much of it one exchange carries.
//!
//! Split from [`oplog_census`](crate::oplog_census) at the file-size
//! ratchet, on the question each half answers. That module answers *what
//! do we hold* — a census read off the op log's offset index, with no op
//! parsed. This one answers *what do we send*, which is a different
//! question with its own failure mode: every bound here exists because an
//! unbounded answer killed something.
//!
//! The budget killed an Android phone with a 253 MB first sync. The
//! windowed gap resend replaced a 164 MB frame that the reading side
//! refused, which stalled a real workspace for eleven hours while its
//! sync dot stayed green. The byte guard is what makes "the frame we
//! build is one the peer can read" true whatever the selection decided.
//!
//! Nothing here opens storage to decide; the census answers first, and
//! only the ops actually selected are read (`JsonlStorage::read_ops_at`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Bound;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::LogOp;
use tracing::{info, warn};

use crate::oplog_census::actor_census;
use crate::protocol::{encode_op, ActorClock, MAX_FRAME_BODY};

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

/// The slice of a gapped actor's log one exchange carries, and where it
/// starts.
///
/// A gap resend used to cross whole, on the reasoning that a budget cuts
/// the oldest ops while the missing ones could be anywhere, so a blind
/// cut can send a slice the peer already holds and make no progress.
/// That reasoning is right and its conclusion was not: "send everything"
/// is not an alternative to a stalled repair when everything does not
/// fit in a frame. It is the same stall, with the repair removed — and
/// on a 164 MB actor it took every *other* actor's ops down with it,
/// because one refused frame fails the whole exchange.
///
/// **The window opens at `count - 1`, and that is where the missing ops
/// are in a real gap.** A gap means the peer holds the watermark and
/// fewer ops beneath it. It gets there by holding a *prefix* plus
/// something newer: every delivery here is oldest-first and ingest is an
/// append, so an interrupted peer keeps a prefix, and the newer op came
/// from another peer or another session. With `count` ops held and one
/// of them the newest, the first one it lacks is at index `count - 1`.
/// The `- 1` also makes a pure prefix (no newer op) re-send one op it
/// already has rather than skipping one it does not; the receiver's
/// ingest dedup absorbs the overlap.
///
/// **A non-prefix subset is real, and the window rotates for it.** A
/// peer can hold the *newest* `count` ops and none of the oldest —
/// `full_actor_resend_converges_and_dedups` is that exact shape, and it
/// is what proved the first version of this window wrong: opening at
/// `count - 1` handed that peer ops it already had, its `(max, count)`
/// came back unchanged, and the repair spun. So when a window does not
/// move the peer's clock, the next one starts where that one ended,
/// wrapping around the log. Progress is then guaranteed for any subset:
/// a full rotation covers every op we hold.
///
/// The cursor is **per process, in memory, keyed by the clock it was
/// answering**. A clock that changed means the peer ingested something,
/// which is progress, so the cursor resets to the good guess; a clock
/// that did not means the window missed, so it advances. Nothing is
/// persisted: a restart costs at most one extra rotation, and a cursor
/// on disk would be sync state about a peer living outside the op log
/// (root `CLAUDE.md` invariant 9) to save exactly that.
fn gap_resend_window(
    hlcs: &BTreeSet<Hlc>,
    actor: ActorId,
    peer: &ActorClock,
    budget: usize,
) -> Vec<Hlc> {
    let below: Vec<Hlc> = hlcs.range(..=peer.max).copied().collect();
    // It all fits: send it all. The cut exists to choose *which* ops
    // cross when they cannot all fit, and skipping any here would be a
    // peer left short for no reason.
    if below.len() <= budget {
        return below;
    }
    let start = next_gap_cursor(actor, peer, below.len(), budget);
    below
        .iter()
        .cycle()
        .skip(start)
        .take(budget)
        .copied()
        .collect()
}

/// Where this exchange's gap window starts, and the only mutable state
/// in this module.
///
/// Keyed on `(actor, peer clock)`: the same clock coming back means the
/// last window taught the peer nothing, so the next one moves past it.
/// A different clock means it ingested, so the cursor returns to
/// `count - 1` — the first op a peer holding a prefix plus the watermark
/// is missing, which is the shape an interrupted ingest leaves.
fn next_gap_cursor(actor: ActorId, peer: &ActorClock, below: usize, budget: usize) -> usize {
    /// The clock a window answered, and where that window started.
    type Answered = (Hlc, u64, usize);
    static CURSORS: OnceLock<Mutex<HashMap<ActorId, Answered>>> = OnceLock::new();
    let guess = (peer.count as usize).saturating_sub(1) % below;
    let Ok(mut cursors) = CURSORS.get_or_init(Mutex::default).lock() else {
        return guess;
    };
    let start = match cursors.get(&actor) {
        Some((max, count, last)) if *max == peer.max && *count == peer.count => {
            (last + budget) % below
        }
        _ => guess,
    };
    cursors.insert(actor, (peer.max, peer.count, start));
    start
}

/// Drop ops from the END of a selection until its encoded form fits in
/// `limit` bytes.
///
/// The count budget bounds how many ops cross; nothing bounds how big
/// one is, and a pasted document is a single `Op::Edit`. This is what
/// makes "the frame we build is always a frame the peer can read" true
/// regardless of what the selection decided — the guarantee the refused
/// 164 MB frame did not have.
///
/// From the end, so what crosses stays the oldest-first prefix the
/// receiver's `(max, count)` depends on. At least one op always crosses:
/// returning nothing would be a silent spin, and an op that alone
/// exceeds a frame is a real limit of this protocol, which the reader
/// then names.
fn fit_in_frame(ops: Vec<LogOp>, limit: usize) -> Vec<LogOp> {
    let mut total = 0usize;
    let mut kept = 0usize;
    for op in &ops {
        // A line is the encoded op plus its newline, matching
        // `encode_ops_blob`'s framing exactly.
        let size = encode_op(op).map(|b| b.len() + 1).unwrap_or(0);
        if kept > 0 && total.saturating_add(size) > limit {
            break;
        }
        total += size;
        kept += 1;
    }
    if kept == ops.len() {
        return ops;
    }
    warn!(
        kept,
        dropped = ops.len() - kept,
        bytes = total,
        "selection trimmed to fit one frame; the rest follows next exchange"
    );
    let mut ops = ops;
    ops.truncate(kept);
    ops
}

/// [`ops_missing_for`] with the budget spelled out, so a test can reach a
/// cut without writing twenty thousand ops.
///
/// Per actor in OUR log:
/// - peer has a GAP below its watermark (it holds fewer distinct ops `<=
///   max_r` than we do) → send a budgeted [`gap_resend_window`];
/// - peer has no entry → send everything we hold for that actor, budgeted;
/// - otherwise → send `ts > max_r`, budgeted.
///
/// **"Peer has no entry" and "peer has a gap" are separate piles, and the
/// budget is why.** Both send from the bottom of what we hold, so they look
/// alike. A peer holding zero ops of an actor cannot already have the slice
/// we cut, so an oldest-first prefix always teaches it something; a gapped
/// peer can, which is what the window rotates for. One `full_resend` set
/// would have to pick one of those two behaviours for both.
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
                    "gap below peer watermark detected; resending a window of that actor's log"
                );
                *actor_id
            })
        })
        .collect();

    let mut gapped: Vec<(ActorId, Hlc)> = Vec::new();
    let mut budgeted: Vec<(ActorId, Hlc)> = Vec::new();
    for (actor_id, hlcs) in &census {
        if gap_resend.contains(actor_id) {
            let peer = peer_clock
                .get(actor_id)
                .expect("a gap implies a peer clock");
            gapped.extend(
                gap_resend_window(hlcs, *actor_id, peer, budget)
                    .into_iter()
                    .map(|ts| (*actor_id, ts)),
            );
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

    let mut picks = gapped;
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
    let ops = storage
        .read_ops_at(&picks)
        .context("read the ops this peer is missing")?;
    // The count budget bounds how MANY ops cross; this bounds how many
    // BYTES, which is what the reader actually refuses. Last, because it
    // is the only place the encoded size is known.
    Ok(fit_in_frame(ops, MAX_FRAME_BODY))
}

#[cfg(test)]
mod tests;
