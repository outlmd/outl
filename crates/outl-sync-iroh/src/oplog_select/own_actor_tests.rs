//! The peer's own actor is never selected.
//!
//! Split from `tests.rs` at the file-size ratchet. A headless node restored
//! on a new machine lost part of its own log while a paired desktop kept
//! it, and the desktop resent a 20,000-op window of it on every exchange,
//! which the node refuses by design. Each test names one branch of the skip.

use super::tests::{census, clock_of, sidecars, write_log};
use super::*;
use std::collections::BTreeSet;

/// The peer's clock for `actor` when it holds the first `held` ops of
/// `ops` plus the newest one: its watermark is the newest and fewer ops
/// sit below it than we hold, which is a gap.
fn gapped_clock(ops: &[LogOp], held: usize) -> ActorClock {
    ActorClock {
        max: ops.last().expect("ops").ts,
        count: held as u64 + 1,
    }
}

/// **A peer's own actor is never selected.**
///
/// A headless node restored from a backup lost some of its own ops while
/// a paired desktop kept them, so the desktop saw `below > count` for the
/// node's actor on every exchange and shipped a 20,000-op window the node
/// refuses by design (it is the only writer of that log). The loop never
/// ended. With the node's own id on the request, the gap for that actor
/// is skipped, while a gap for any *other* actor in the same call is
/// resent as before.
#[test]
fn the_peers_own_actor_is_not_gap_resent_but_other_gaps_still_are() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (p, q) = (ActorId::new(), ActorId::new());
    let p_ops = write_log(&ops_dir, p, 12);
    let q_ops = write_log(&ops_dir, q, 12);
    let peer_clock = HashMap::from([(p, gapped_clock(&p_ops, 3)), (q, gapped_clock(&q_ops, 3))]);

    let sent = ops_missing_for(&ops_dir, &census(&ops_dir), &peer_clock, Some(p)).expect("delta");

    assert!(
        sent.iter().all(|op| op.actor != p),
        "the peer refuses its own actor's ops, so none may cross"
    );
    assert!(
        sent.iter().any(|op| op.actor == q),
        "a gap for another actor is selected exactly as before"
    );
}

/// The same setup with no origin (a peer that predates the field) keeps
/// the old behaviour: the gap for P is resent.
#[test]
fn without_an_origin_the_same_gap_is_still_resent() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (p, q) = (ActorId::new(), ActorId::new());
    let p_ops = write_log(&ops_dir, p, 12);
    let q_ops = write_log(&ops_dir, q, 12);
    let peer_clock = HashMap::from([(p, gapped_clock(&p_ops, 3)), (q, gapped_clock(&q_ops, 3))]);

    let sent = ops_missing_for(&ops_dir, &census(&ops_dir), &peer_clock, None).expect("delta");

    assert!(sent.iter().any(|op| op.actor == p), "P is gap-resent");
    assert!(sent.iter().any(|op| op.actor == q), "Q is gap-resent");
}

/// The fast path is skipped too: ops of P newer than the peer's max for
/// P are ops the peer would refuse just the same.
#[test]
fn the_peers_own_actor_is_not_sent_on_the_fast_path_either() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (p, q) = (ActorId::new(), ActorId::new());
    let p_ops = write_log(&ops_dir, p, 9);
    let q_ops = write_log(&ops_dir, q, 9);
    // Both peers' clocks are gapless and behind ours: a pure `after(max)`.
    let held = |ops: &[LogOp]| clock_of(&ops[..3].iter().map(|op| op.ts).collect());
    let peer_clock = HashMap::from([(p, held(&p_ops)), (q, held(&q_ops))]);

    let sent = ops_missing_for(&ops_dir, &census(&ops_dir), &peer_clock, Some(p)).expect("delta");

    assert!(sent.iter().all(|op| op.actor != p));
    assert_eq!(
        sent.iter().map(|op| op.ts).collect::<BTreeSet<_>>(),
        q_ops[3..].iter().map(|op| op.ts).collect::<BTreeSet<_>>(),
        "every other actor crosses exactly as before"
    );
}

/// A peer with no entry for its own actor would otherwise be sent our
/// whole copy of it, which it refuses just the same.
#[test]
fn a_peer_with_no_clock_for_its_own_actor_is_not_sent_that_log() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (p, q) = (ActorId::new(), ActorId::new());
    write_log(&ops_dir, p, 5);
    let q_ops = write_log(&ops_dir, q, 5);

    let sent =
        ops_missing_for(&ops_dir, &census(&ops_dir), &HashMap::new(), Some(p)).expect("delta");

    assert_eq!(
        sent.iter().map(|op| op.ts).collect::<BTreeSet<_>>(),
        q_ops.iter().map(|op| op.ts).collect::<BTreeSet<_>>()
    );
}

/// The skip must not reopen storage: it is a branch of the selection, and
/// "an early return is not a budget" (root `CLAUDE.md` invariant 15).
#[test]
fn skipping_the_peers_own_actor_still_costs_no_open() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (p, q) = (ActorId::new(), ActorId::new());
    let p_ops = write_log(&ops_dir, p, 12);
    write_log(&ops_dir, q, 4);
    let peer_clock = HashMap::from([(p, gapped_clock(&p_ops, 3))]);

    let sent = ops_missing_for(&ops_dir, &census(&ops_dir), &peer_clock, Some(p)).expect("delta");
    assert_eq!(sent.len(), 4, "only Q's ops cross");
    for actor in [p, q] {
        for path in sidecars(&ops_dir, actor) {
            assert!(!path.exists(), "{} came back", path.display());
        }
    }
}
