//! Does the device that **answers** a dial know it was reached?
//!
//! `peer_health()` is what every GUI sync dot reads, and it used to be written
//! by outbound dials alone. Reachability is not symmetric and outl leans on
//! that: a phone behind carrier NAT is the side that dials (mobile forces a
//! pass every 3s for exactly this reason), so the desktop's dials to it fail
//! while its own serves succeed. The dot therefore read "no peer reachable"
//! one line above "Last synced just now", over a peer whose ops had just
//! landed on disk ([#575](https://github.com/outlmd/outl/issues/575)).
//!
//! The responder's verdict has to be earned, not assumed: the first test is
//! the fix, the second is the thing that must not come with it, since a
//! dialer `authz` refuses is not a peer at all and painting the dot green for
//! a revoked device would be the worse bug.

// `common` is shared by every integration test; this one uses a subset.
#[allow(dead_code)]
mod common;

use std::sync::mpsc;
use std::time::Duration;

use outl_core::id::ActorId;
use outl_sync_iroh::test_support::{self, HealthWindow};

use common::{fresh_identity, seed_ops, shared_wid, STEP_TIMEOUT};

/// Wait for the responder to record `peer`, or give up.
///
/// The serve runs in its own task, so the initiator's `run_delta_sync`
/// returning says nothing about where that task is. Polling observes the
/// serve side without asserting an ordering the production code does not
/// promise — and it is what caught the first version of this fix, which
/// recorded after the durable-ingest ack and was therefore skipped whenever
/// `send.finish()` errored on an initiator that had already hung up.
async fn wait_reachable(health: &HealthWindow, peer: iroh::EndpointId) -> bool {
    for _ in 0..200 {
        if health.reachable(peer) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// A completed inbound exchange marks the dialer reachable on the responder.
///
/// This is the desktop's side of the NAT pair: it never successfully dials the
/// phone, so without this the only evidence it has of a live peer is the one
/// it was ignoring.
#[tokio::test(flavor = "multi_thread")]
async fn an_inbound_sync_marks_the_dialer_reachable_on_the_responder() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A");
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B");
    let b_addr = ep_b.addr();

    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let (_router_b, health_b) = test_support::spawn_responder_with_health(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    assert!(
        !health_b.reachable(id_a.node_id()),
        "B has not met A yet; a peer it never exchanged with must not read reachable"
    );

    seed_ops(dir_a.path(), actor_a, 3);
    let (tx, _rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(&ep_a, b_addr, dir_a.path(), &shared_wid(), actor_a, tx),
    )
    .await
    .expect("sync did not finish in time")
    .expect("sync must succeed");

    assert!(
        wait_reachable(&health_b, id_a.node_id()).await,
        "B ingested A's ops on an inbound serve, so A is reachable from B's own \
         evidence; reading it as offline is what kept the desktop sync dot orange \
         while sync was working (#575)"
    );
    assert!(
        health_b.snapshot().iter().all(|h| h.last_rtt_ms.is_none()),
        "a serve is not a round trip: no rtt was measured here, so none may be reported"
    );
}

/// A dialer the responder refuses is never recorded as reachable.
///
/// `authorize_or_close` returns before the exchange, so the recording sits
/// behind it. A revoked device that can still open a QUIC connection must not
/// be able to paint this device's dot green.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_dialer_is_never_marked_reachable() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A");
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B");
    let b_addr = ep_b.addr();

    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    // Nobody authorized: B trusts no device at all.
    let (_router_b, health_b) = test_support::spawn_responder_with_health(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[],
    );

    seed_ops(dir_a.path(), actor_a, 2);
    let (tx, _rx) = mpsc::channel::<()>();
    let _ = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(&ep_a, b_addr, dir_a.path(), &shared_wid(), actor_a, tx),
    )
    .await
    .expect("the refused sync must not hang");

    // Give the responder the same window the passing case gets, so this is a
    // verdict rather than a race it happened to win.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !health_b.reachable(id_a.node_id()),
        "A was refused by authz, so it is not a peer; recording it as reachable \
         would let a revoked device decide what this device's sync dot says"
    );
}

/// A dial that connected and then broke mid-exchange still counts as contact.
///
/// This is the shape the field report had: an inbound serve from grokbot at
/// 19:38:30 marked it reachable, and the outbound dial to that same peer
/// failed at 19:38:34 on `send our ops blob` — connection open, push cut off.
/// Recording that as "offline" flipped the dot back to orange four seconds
/// after it went green, on a peer that was plainly answering.
#[tokio::test(flavor = "multi_thread")]
async fn an_exchange_that_breaks_after_connecting_still_counts_as_contact() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_a.path(), "b");
    let actor_a = ActorId::new();

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A");
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B");
    let b_addr = ep_b.addr();
    // Completes the exchange and never confirms durable ingest — a peer whose
    // OS suspended it mid-push.
    let _router_b = test_support::spawn_half_responder(ep_b);

    seed_ops(dir_a.path(), actor_a, 2);
    let health_a = test_support::HealthWindow::default();
    let (tx, _rx) = mpsc::channel::<()>();
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_tracking_health(
            &health_a,
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            tx,
        ),
    )
    .await
    .expect("the sync must not hang");

    assert!(
        result.is_err(),
        "the half responder never confirms, so the pass must fail — if this \
         passes the test is no longer about a broken exchange"
    );
    assert!(
        health_a.reachable(id_b.node_id()),
        "the connection opened, so the peer is reachable; reading a failed \
         exchange as 'offline' is what made the dot flicker (#575)"
    );
}

/// A dial that never connected is offline, which is the whole point of the
/// distinction above.
#[tokio::test(flavor = "multi_thread")]
async fn a_dial_that_never_connects_reads_as_offline() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_a.path(), "b");
    let actor_a = ActorId::new();

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A");
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B");
    let b_addr = ep_b.addr();
    // Take B off the air, keeping the address A will dial.
    ep_b.close().await;

    seed_ops(dir_a.path(), actor_a, 1);
    let health_a = test_support::HealthWindow::default();
    let (tx, _rx) = mpsc::channel::<()>();
    let _ = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_tracking_health(
            &health_a,
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            tx,
        ),
    )
    .await
    .expect("the failed dial must not hang");

    assert!(
        !health_a.reachable(id_b.node_id()),
        "nothing answered, so there is no contact to report"
    );
}
