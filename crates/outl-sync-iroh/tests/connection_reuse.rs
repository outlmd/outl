//! Does the connection actually survive a sync, and is it actually reused?
//!
//! These exist because the claim "syncs got faster" was, at the time it was
//! made, an inference from a device log rather than a measurement. The three
//! tests below check the mechanism instead of the wall clock, so they mean the
//! same thing on a fast laptop and in CI:
//!
//! 1. after a successful sync the connection is still open (the v3 ack moved
//!    off the close code, which is what makes the rest possible);
//! 2. the second sync to a peer runs on the SAME connection (`stable_id`), not
//!    a fresh one;
//! 3. a broken connection is dropped rather than handed out again.
//!
//! Timing is reported for information but never asserted on. Over loopback a
//! connect is sub-millisecond, so a timing assertion here would prove nothing
//! about the ~15s relay connect it is meant to eliminate on a real network.

// `common` is shared by every integration test; this one uses a subset.
#[allow(dead_code)]
mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use outl_core::id::ActorId;
use outl_sync_iroh::test_support;

use common::{fresh_identity, seed_ops, shared_wid, STEP_TIMEOUT};

/// Stand up a responder B and return everything an initiator A needs.
async fn paired_pair() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    ActorId,
    iroh::Endpoint,
    iroh::EndpointAddr,
    iroh::protocol::Router,
) {
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
    let router_b = test_support::spawn_responder(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    (dir_a, dir_b, actor_a, ep_a, b_addr, router_b)
}

/// The v3 ack travels on the stream, so a confirmed sync leaves the connection
/// **open**. Under v2 the confirmation WAS the close, so this was impossible by
/// construction — and that is why every sync paid a fresh connect.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirmed_sync_leaves_the_connection_open() {
    let (dir_a, _dir_b, actor_a, ep_a, b_addr, _router_b) = paired_pair().await;
    seed_ops(dir_a.path(), actor_a, 3);

    let conns = test_support::connection_pool(ep_a.clone());
    let (tx, _rx) = mpsc::channel::<()>();

    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_pooled(
            &conns,
            b_addr.clone(),
            dir_a.path(),
            &shared_wid(),
            actor_a,
            tx,
        ),
    )
    .await
    .expect("sync did not finish in time")
    .expect("sync must succeed");

    let conn = test_support::pooled_connection(&conns, b_addr.id)
        .expect("a confirmed sync must leave a connection in the pool");
    assert!(
        conn.close_reason().is_none(),
        "the connection must still be usable after a confirmed sync; \
         if this fails the ack has moved back onto the close code and every \
         sync is paying a fresh connect again"
    );
}

/// The second sync runs on the SAME connection.
///
/// `stable_id` is per-connection, so equal ids mean one connection served both
/// exchanges. This is the actual claim behind "syncs got faster" — asserted on
/// identity rather than on a stopwatch, because loopback would hide the effect
/// the change exists for.
#[tokio::test(flavor = "multi_thread")]
async fn the_second_sync_reuses_the_first_connection() {
    let (dir_a, _dir_b, actor_a, ep_a, b_addr, _router_b) = paired_pair().await;
    seed_ops(dir_a.path(), actor_a, 3);

    let conns = test_support::connection_pool(ep_a.clone());
    let (tx, _rx) = mpsc::channel::<()>();

    let mut ids = Vec::new();
    let mut timings = Vec::new();
    for pass in 0..3 {
        // A new op each round so the pass has something to push, not a no-op.
        seed_ops(dir_a.path(), actor_a, 1);
        let started = Instant::now();
        tokio::time::timeout(
            STEP_TIMEOUT,
            test_support::run_delta_sync_pooled(
                &conns,
                b_addr.clone(),
                dir_a.path(),
                &shared_wid(),
                actor_a,
                tx.clone(),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("sync {pass} did not finish in time"))
        .unwrap_or_else(|e| panic!("sync {pass} failed: {e:#}"));
        timings.push(started.elapsed());
        ids.push(
            test_support::pooled_connection(&conns, b_addr.id)
                .expect("pool must hold a connection")
                .stable_id(),
        );
    }

    // Informational only — see the module docs on why this is not asserted.
    eprintln!("sync timings (loopback): {timings:?}");

    assert_eq!(
        ids[0], ids[1],
        "the second sync must run on the first sync's connection, not a new one"
    );
    assert_eq!(ids[1], ids[2], "and so must the third");
}

/// A connection the peer has torn down is dropped, not handed out again.
///
/// Without this the pool is worse than no pool: one dead entry would fail
/// every future sync to that peer for the life of the process, with no way out
/// but a restart.
#[tokio::test(flavor = "multi_thread")]
async fn a_dead_connection_is_replaced_rather_than_reused() {
    let (dir_a, _dir_b, actor_a, ep_a, b_addr, router_b) = paired_pair().await;
    seed_ops(dir_a.path(), actor_a, 2);

    let conns = test_support::connection_pool(ep_a.clone());
    let (tx, _rx) = mpsc::channel::<()>();

    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_pooled(
            &conns,
            b_addr.clone(),
            dir_a.path(),
            &shared_wid(),
            actor_a,
            tx.clone(),
        ),
    )
    .await
    .expect("first sync did not finish in time")
    .expect("first sync must succeed");

    let first_id = test_support::pooled_connection(&conns, b_addr.id)
        .expect("pool must hold a connection")
        .stable_id();

    // The peer goes away. The pooled connection is now unusable.
    router_b.shutdown().await.ok();
    // Give the close frame time to land so `close_reason()` is populated.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let stale = test_support::pooled_connection(&conns, b_addr.id);
    assert!(
        stale.is_none() || stale.expect("checked").close_reason().is_some(),
        "a connection whose peer shut down must not be reported as live"
    );

    // The next sync fails (the peer is gone) but must not leave the dead
    // connection cached — the pool has to be usable again the moment the peer
    // comes back.
    let _ = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync_pooled(
            &conns,
            b_addr.clone(),
            dir_a.path(),
            &shared_wid(),
            actor_a,
            tx,
        ),
    )
    .await;

    if let Some(conn) = test_support::pooled_connection(&conns, b_addr.id) {
        assert_ne!(
            conn.stable_id(),
            first_id,
            "the dead connection must have been replaced, not kept"
        );
    }
}
