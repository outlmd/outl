//! The forced sync pass (`sync_now`) over real QUIC on loopback.
//!
//! Every mobile background window waits on this pass, and the iOS
//! `BGAppRefreshTask` caps it at 12s. One unreachable peer costs up to 15s
//! (5s direct + 10s relay fallback), so a pass that dials peers one after
//! another never reaches a live peer listed after a dead one inside that
//! window. These tests pin that a dead peer does not delay the others.

mod common;

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use outl_core::id::ActorId;
use outl_sync_iroh::test_support;

use common::{disk_has_all_nodes, fresh_identity, seed_ops, shared_wid, wait_until, STEP_TIMEOUT};

/// Less than the 5s a single direct connect attempt is allowed to hang, so a
/// pass that waits for the dead peer before dialing the live one cannot pass.
const LIVE_PEER_DEADLINE: Duration = Duration::from_secs(4);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_peer_does_not_hold_back_a_live_one() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let dir_dead = tempfile::tempdir().expect("dead peer tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let a_nodes = seed_ops(dir_a.path(), actor_a, 3);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");
    let id_dead = fresh_identity(dir_dead.path(), "dead");

    // A: a live, paired responder.
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let a_addr = ep_a.addr();
    let _router_a = test_support::spawn_responder(
        ep_a,
        dir_a.path().to_path_buf(),
        shared_wid(),
        actor_a,
        a_ready_tx,
        &[id_b.node_id()],
    );

    // The dead peer: a socket that is bound, so nothing refuses the packets,
    // and never answers, like a laptop that went to sleep. A QUIC handshake
    // to it can only time out.
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind silent socket");
    let dead_addr = iroh::EndpointAddr::new(id_dead.node_id())
        .with_ip_addr(silent.local_addr().expect("silent socket addr"));

    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();

    // The dead peer goes FIRST, the way a laptop paired before the always-on
    // node sits at the head of `peers.json`.
    let started = Instant::now();
    let pass = tokio::spawn(test_support::run_force_sync(
        ep_b,
        vec![dead_addr, a_addr],
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
    ));

    let a_node_strs: BTreeSet<String> = a_nodes.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &a_node_strs)
        }),
        "B should pull A's ops in the forced pass"
    );
    let live_synced = started.elapsed();
    assert!(
        live_synced < LIVE_PEER_DEADLINE,
        "the live peer synced after {live_synced:?}; a dead peer ahead of it in the list \
         must not delay it (the pass dials peers concurrently)"
    );

    // The pass still waits for the dead dial to give up, and does give up.
    tokio::time::timeout(STEP_TIMEOUT, pass)
        .await
        .expect("the forced pass must finish once the dead dial times out")
        .expect("forced pass task");
    drop(silent);
}
