//! What one exchange selects, and the three bounds on it.
//!
//! Split out of `oplog_select.rs` the way `outl-md`'s `parse/tests.rs`
//! was split out of its grammar: the module stays one piece, the file
//! stays under the size guard.
//!
//! Most of these are named after a bug. The budget tests are the
//! Android first-sync crash; the window tests are the 164 MB frame that
//! a reading peer refused, which stalled a workspace in one direction
//! for eleven hours.

use super::*;
use crate::oplog_census::local_vector_clock;
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

/// The vector clock a peer would report after holding exactly `held`.
fn clock_of(held: &BTreeSet<Hlc>) -> ActorClock {
    ActorClock {
        max: *held.last().expect("a non-empty holding"),
        count: held.len() as u64,
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
/// has a gap underneath it, and ops still cross — the early-out must
/// not swallow that, since nothing sits above the max.
///
/// It used to assert the FULL log crosses. The resend is windowed now
/// (see `gap_resend_window`), so what this pins is the property that
/// mattered: a gap with nothing above the watermark is not read as
/// "converged", and what crosses is the ops the peer lacks under
/// either shape a real gap takes.
#[test]
fn a_gap_below_the_peers_watermark_still_sends_what_the_peer_lacks() {
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

    let sent: BTreeSet<Hlc> = ops_missing_for(&ops_dir, actor, &peer_clock)
        .expect("delta")
        .iter()
        .map(|op| op.ts)
        .collect();

    assert!(
        !sent.is_empty(),
        "a gap under the watermark is not convergence"
    );
    // A prefix of 2: ops 2, 3 and 4 are missing.
    for op in &ops[2..] {
        assert!(sent.contains(&op.ts), "a prefix-shaped gap is covered");
    }
    // A prefix of 1 plus the newest: op 1, 2 and 3 are missing.
    for op in &ops[1..4] {
        assert!(sent.contains(&op.ts), "a prefix-plus-newest gap is covered");
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

/// A gap resend obeys the budget, because the alternative was not
/// "a large frame" — it was **no frame at all**.
///
/// This test used to assert the opposite, and the reasoning behind it
/// was sound except for its last step. A budget cuts the oldest ops, a
/// gap resend exists because we cannot tell which ops below the peer's
/// watermark are missing, so a blind cut can send a slice the peer
/// already holds and make no progress. True. The step that does not
/// follow is "therefore send everything": an oversized resend is
/// refused by `MAX_FRAME_BODY` on the reading side, which is not a
/// loud failure in any sense the user can act on — it is the same
/// non-convergence, minus the repair.
///
/// Measured, on the maintainer's own workspace: an actor holding
/// 508,421 ops (164 MB) against a 64 MB frame cap. Every exchange
/// ingested the peer's ops, built that frame, had it refused, and
/// never reached the durable-ingest ack — so **no** actor's ops
/// crossed, for eleven hours, while the sync dot stayed green. One
/// actor with a gap took the whole workspace down.
///
/// So the budget applies, and `gap_resend_window` is what makes the
/// cut progress instead of spinning. See its doc for the window and
/// for the one shape that still needs several rounds.
#[test]
fn a_gap_resend_obeys_the_budget_so_the_frame_is_always_sendable() {
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
    assert!(
        sent.len() <= 5,
        "a gap resend that cannot be framed is a repair that never runs"
    );
    assert!(!sent.is_empty(), "a gap resend must send something");
}

/// The window starts where the peer stopped, so the common gap is
/// repaired by the very first exchange rather than after a walk.
///
/// "Prefix plus the newest op" is the shape a real gap has: every
/// delivery is oldest-first and ingest is an append, so a peer
/// interrupted mid-ingest holds a prefix, and the watermark it is
/// measured against arrived from somewhere else.
#[test]
fn the_window_opens_on_the_ops_the_peer_is_actually_missing() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let ops = write_log(&ops_dir, actor, 30);

    // 20 oldest, plus the newest: 21 ops, a hole from index 20 to 28.
    let peer_clock: HashMap<ActorId, ActorClock> = [(
        actor,
        ActorClock {
            max: ops.last().expect("op").ts,
            count: 21,
        },
    )]
    .into_iter()
    .collect();

    let sent = ops_missing_for_capped(&ops_dir, actor, &peer_clock, 5).expect("delta");
    let first = sent.first().expect("a window").ts;
    assert_eq!(
        first, ops[20].ts,
        "the window opens on the first op the peer does not hold, not on the log's start"
    );
}

/// The whole point of cutting it: repeated exchanges close the gap.
///
/// Drives the real loop — the peer reports `(max, count)`, ingests
/// what arrives, reports again — against a budget far smaller than the
/// hole, and requires it to converge.
#[test]
fn a_budgeted_gap_resend_converges_over_repeated_exchanges() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let ops = write_log(&ops_dir, actor, 60);
    let everything: BTreeSet<Hlc> = ops.iter().map(|op| op.ts).collect();

    // A prefix of 5, plus the newest: the gap is 54 ops wide.
    let mut held: BTreeSet<Hlc> = ops[..5].iter().map(|op| op.ts).collect();
    held.insert(ops.last().expect("op").ts);

    for round in 0..100 {
        let peer_clock: HashMap<ActorId, ActorClock> =
            [(actor, clock_of(&held))].into_iter().collect();
        let sent = ops_missing_for_capped(&ops_dir, actor, &peer_clock, 7).expect("delta");
        if sent.is_empty() {
            assert_eq!(
                held, everything,
                "a quiet round must mean the gap is closed"
            );
            return;
        }
        assert!(sent.len() <= 7, "round {round} broke the budget");
        for op in sent {
            held.insert(op.ts);
        }
    }
    panic!("the gap never closed");
}

/// The shape that proved the first window wrong, driven to
/// convergence.
///
/// A peer holding the **newest** ops and none of the oldest gets a
/// window it already has, so its clock comes back unchanged. That is
/// the signal the cursor reads: unchanged means the window missed,
/// so the next one moves past it, around the log, until it lands on
/// something the peer lacks.
///
/// `full_actor_resend_converges_and_dedups` (tests/regression.rs) is
/// this same shape over the real transport, which is where it was
/// caught — a unit test asserting the old "send everything" could
/// not see it.
#[test]
fn a_window_that_teaches_the_peer_nothing_moves_on() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let ops = write_log(&ops_dir, actor, 60);
    let everything: BTreeSet<Hlc> = ops.iter().map(|op| op.ts).collect();

    // The newest 40 and none of the oldest 20: every guess that opens
    // near `count` lands on ops the peer already holds.
    let mut held: BTreeSet<Hlc> = ops[20..].iter().map(|op| op.ts).collect();

    for round in 0..100 {
        let peer_clock: HashMap<ActorId, ActorClock> =
            [(actor, clock_of(&held))].into_iter().collect();
        let sent = ops_missing_for_capped(&ops_dir, actor, &peer_clock, 7).expect("delta");
        if sent.is_empty() {
            assert_eq!(held, everything, "a quiet round must mean convergence");
            return;
        }
        assert!(sent.len() <= 7, "round {round} broke the budget");
        for op in sent {
            held.insert(op.ts);
        }
    }
    panic!("the rotating window never reached the missing prefix");
}

/// A frame is bounded by bytes, not by op count, and nothing promises
/// an op is small: a pasted document is one `Op::Edit`.
///
/// So the byte guard is the one that makes "the frame is always
/// sendable" true, whatever the selection decided. It cuts from the
/// END, keeping the oldest-first prefix the receiver's clock depends
/// on.
#[test]
fn an_oversized_selection_is_cut_to_what_a_frame_can_carry() {
    let ops = {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        write_log(&ops_dir, ActorId::new(), 8)
    };
    // A budget in bytes that only the first few ops fit inside.
    let one = encode_op(&ops[0]).expect("encode").len() + 1;
    let kept = fit_in_frame(ops.clone(), one * 3);

    assert_eq!(kept.len(), 3, "the cut lands on the byte budget");
    assert_eq!(
        kept.iter().map(|op| op.ts).collect::<Vec<_>>(),
        ops[..3].iter().map(|op| op.ts).collect::<Vec<_>>(),
        "the cut takes from the end, so what crosses stays a prefix"
    );
    assert_eq!(
        fit_in_frame(ops.clone(), usize::MAX).len(),
        ops.len(),
        "a selection that already fits is untouched"
    );
}

/// An empty result from the byte guard would be a silent stall, so one
/// op always crosses even when it alone is over the cap.
///
/// It will then be refused by the reader, which is the honest outcome:
/// a single op larger than a frame is a limit this protocol genuinely
/// has, and `MAX_FRAME_BODY` is 64 MB against `outl-core`'s own op
/// ceiling. Returning nothing instead would spin forever with nothing
/// in any log to say why.
#[test]
fn the_byte_guard_never_returns_an_empty_selection() {
    let ops = {
        let tmp = tempfile::tempdir().expect("tmp");
        let ops_dir = tmp.path().join("ops");
        write_log(&ops_dir, ActorId::new(), 3)
    };
    assert_eq!(fit_in_frame(ops, 1).len(), 1);
}
