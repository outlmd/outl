//! Integration tests for `outl-sync-iroh` over real QUIC on loopback.
//!
//! Two cases run the actual code paths end to end:
//!
//! 1. `pairing_roundtrip` — `host_pairing` + `join_pairing` exchange identities
//!    and both `peers.json` files end up holding the other node's id.
//! 2. `bidirectional_delta_sync` — two workspaces, each with its own actor's
//!    ops, reconcile over one connection so BOTH sides hold ALL ops from BOTH
//!    actors. This is the regression guard for "B never receives A's offline
//!    ops".
//! 3. `offline_catchup` — A gains new ops while B is already up; a single
//!    connection (initiated by A) lands them on B without B asking.
//!
//! Determinism: every network step is wrapped in a generous `tokio` timeout so
//! a hung handshake fails loudly instead of hanging CI. Connections use the
//! full `EndpointAddr` from `endpoint.addr()` (direct loopback addrs), so no
//! relay or n0 discovery round-trip is on the happy path.

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::Duration;

use outl_core::id::ActorId;
use outl_core::WorkspaceId;
use outl_sync_iroh::{
    decode_ticket, host_pairing, join_pairing, mint_ticket, test_support, IrohIdentity, PeersStore,
    WorkspaceAdoption,
};

mod common;

use common::{
    created_nodes_on_disk, disk_has_all_nodes, fresh_identity, seed_ops, shared_wid, wait_until,
    STEP_TIMEOUT,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_roundtrip() {
    let host_dir = tempfile::tempdir().expect("host tempdir");
    let join_dir = tempfile::tempdir().expect("join tempdir");

    let host_identity = fresh_identity(host_dir.path(), "host");
    let join_identity = fresh_identity(join_dir.path(), "join");

    let host_node_id = host_identity.node_id().to_string();
    let join_node_id = join_identity.node_id().to_string();

    let host_peers = host_dir.path().join("peers.json");
    let join_peers = join_dir.path().join("peers.json");

    // The pairing functions read/write `<root>/.outl/workspace-id`; the tempdir
    // is the workspace root here (peers.json sits directly in it).
    let host_root = host_dir.path().to_path_buf();
    let join_root = join_dir.path().to_path_buf();

    // Channel to hand the host's ticket to the joiner once it's printed.
    let (ticket_tx, ticket_rx) = tokio::sync::oneshot::channel::<String>();

    let host_peers_for_task = host_peers.clone();
    let host_task = tokio::spawn(async move {
        let mut ticket_tx = Some(ticket_tx);
        tokio::time::timeout(
            STEP_TIMEOUT,
            host_pairing(
                host_identity,
                &host_peers_for_task,
                &host_root,
                Some("host-device".into()),
                |ticket, _qr| {
                    // Fires synchronously inside host_pairing before it blocks
                    // on accept(); forward the ticket to the joiner.
                    if let Some(tx) = ticket_tx.take() {
                        let _ = tx.send(ticket.to_string());
                    }
                },
            ),
        )
        .await
        .expect("host pairing timed out")
        .expect("host pairing failed")
    });

    let join_peers_for_task = join_peers.clone();
    let join_task = tokio::spawn(async move {
        let ticket = ticket_rx.await.expect("never received host ticket");
        tokio::time::timeout(
            STEP_TIMEOUT,
            join_pairing(
                join_identity,
                &ticket,
                &join_peers_for_task,
                &join_root,
                Some("join-device".into()),
            ),
        )
        .await
        .expect("join pairing timed out")
        .expect("join pairing failed")
    });

    let host_entry = host_task.await.expect("host task panicked");
    let (join_entry, adoption) = join_task.await.expect("join task panicked");

    // The joiner adopted the host's workspace id (issue #197): both sides now
    // share one id on disk, so later delta-sync passes the workspace-id check
    // instead of being rejected as a mismatch.
    let host_wid = WorkspaceId::read_or_create(host_dir.path()).expect("host wid");
    let join_wid = WorkspaceId::read_or_create(join_dir.path()).expect("join wid");
    assert_eq!(
        host_wid, join_wid,
        "joiner must adopt the host's workspace id"
    );
    assert_eq!(
        adoption,
        WorkspaceAdoption::Adopted(host_wid),
        "join_pairing should report the host id it adopted"
    );

    // Each side persisted the OTHER side's node id.
    assert_eq!(
        host_entry.node_id, join_node_id,
        "host should have stored the joiner's node id"
    );
    assert_eq!(
        join_entry.node_id, host_node_id,
        "joiner should have stored the host's node id"
    );

    // And it landed in both peers.json files.
    let host_store = PeersStore::load_or_default(&host_peers).expect("load host peers");
    let join_store = PeersStore::load_or_default(&join_peers).expect("load join peers");

    assert!(
        host_store.list().iter().any(|p| p.node_id == join_node_id),
        "host peers.json missing joiner node id"
    );
    assert!(
        join_store.list().iter().any(|p| p.node_id == host_node_id),
        "join peers.json missing host node id"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gui_pairing_over_live_sync_endpoint() {
    // The GUI consolidation: pairing rides the LIVE sync endpoint (no second
    // endpoint with the device identity). Two real `IrohSyncTransport`s are
    // started over loopback; the host arms via `pair_host`, the joiner dials via
    // `pair_join`, and both peers.json end up holding the other's node id —
    // proving the router's PAIRING_ALPN handler + `pair_join` path work without
    // binding a separate pairing endpoint.
    let host_dir = tempfile::tempdir().expect("host tempdir");
    let join_dir = tempfile::tempdir().expect("join tempdir");

    let host_identity = IrohIdentity::load_or_generate(&host_dir.path().join("identity.key"))
        .expect("host identity");
    let join_identity = IrohIdentity::load_or_generate(&join_dir.path().join("identity.key"))
        .expect("join identity");
    let host_node_id = host_identity.node_id().to_string();
    let join_node_id = join_identity.node_id().to_string();

    let host_peers = host_dir.path().join("peers.json");
    let join_peers = join_dir.path().join("peers.json");

    let host = outl_sync_iroh::IrohSyncTransport::new(
        host_identity,
        PeersStore::load_or_default(&host_peers).expect("host peers"),
        None,
    );
    let join = outl_sync_iroh::IrohSyncTransport::new(
        join_identity,
        PeersStore::load_or_default(&join_peers).expect("join peers"),
        None,
    );

    // Start both transports (binds the one long-lived endpoint each, mounts the
    // PAIRING_ALPN handler). `start` writes peers.json into <dir> via the store
    // path it was constructed with.
    use outl_actions::SyncTransport;
    let (htx, _hrx) = mpsc::channel::<()>();
    let (jtx, _jrx) = mpsc::channel::<()>();
    host.start(host_dir.path().to_path_buf(), ActorId::new(), htx);
    join.start(join_dir.path().to_path_buf(), ActorId::new(), jtx);

    // Wait for both endpoints to bind + publish their pairing hub. `pair_host`
    // / `pair_join` error with "not started yet" until `run_iroh` publishes the
    // hub, so poll until the host stops returning that.
    wait_for_pairing_ready(&host).await;
    wait_for_pairing_ready(&join).await;

    // Host arms and hands us the ticket via the callback channel.
    let (ticket_tx, ticket_rx) = tokio::sync::oneshot::channel::<String>();
    let host_for_task = host.clone();
    let host_task = tokio::spawn(async move {
        let mut ticket_tx = Some(ticket_tx);
        tokio::time::timeout(
            STEP_TIMEOUT,
            host_for_task.pair_host(Some("host-device".into()), move |ticket: &str| {
                if let Some(tx) = ticket_tx.take() {
                    let _ = tx.send(ticket.to_string());
                }
            }),
        )
        .await
        .expect("host pair timed out")
        .expect("host pair failed")
    });

    let ticket = tokio::time::timeout(STEP_TIMEOUT, ticket_rx)
        .await
        .expect("ticket timed out")
        .expect("never received ticket");

    let join_entry = tokio::time::timeout(
        STEP_TIMEOUT,
        join.pair_join(ticket, Some("join-device".into())),
    )
    .await
    .expect("join pair timed out")
    .expect("join pair failed");
    let host_entry = host_task.await.expect("host task panicked");

    assert_eq!(
        host_entry.node_id, join_node_id,
        "host stored the joiner's node id via the live endpoint"
    );
    assert_eq!(
        join_entry.node_id, host_node_id,
        "joiner stored the host's node id via the live endpoint"
    );

    // Both files landed the peer (persisted by the hub, not a one-shot endpoint).
    let host_store = PeersStore::load_or_default(&host_peers).expect("reload host peers");
    let join_store = PeersStore::load_or_default(&join_peers).expect("reload join peers");
    assert!(
        host_store.list().iter().any(|p| p.node_id == join_node_id),
        "host peers.json missing joiner"
    );
    assert!(
        join_store.list().iter().any(|p| p.node_id == host_node_id),
        "join peers.json missing host"
    );

    host.shutdown();
    join.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidirectional_delta_sync() {
    // Two devices, two actors, two op logs that start fully disjoint.
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    let a_nodes = seed_ops(dir_a.path(), actor_a, 3);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 4);

    // Sanity: before sync, neither side knows the other's ops.
    let a_before = created_nodes_on_disk(dir_a.path(), actor_a);
    let b_before = created_nodes_on_disk(dir_b.path(), actor_b);
    assert_eq!(a_before.len(), 3, "A starts with only its own ops");
    assert_eq!(b_before.len(), 4, "B starts with only its own ops");

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // B is the responder: bind an endpoint, mount the real protocol handler.
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_responder(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    // A is the initiator: bind an endpoint and run the real delta_sync against
    // B's full EndpointAddr (direct loopback addrs — no relay on the path).
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");

    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    // Give B's responder task a beat to finish persisting (serve() ingests the
    // initiator's push after the initiator returns).
    let all_nodes: BTreeSet<String> = a_nodes
        .iter()
        .chain(b_nodes.iter())
        .map(|n| n.to_string())
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &all_nodes)
                && disk_has_all_nodes(dir_b.path(), actor_b, &all_nodes)
        }),
        "both sides should hold all 7 ops after sync"
    );

    // Explicit assertions on the merged set: A learned B's ops AND B learned A's.
    let a_after = created_nodes_on_disk(dir_a.path(), actor_a);
    let b_after = created_nodes_on_disk(dir_b.path(), actor_b);

    let actor_a_str = actor_a.to_string();
    let actor_b_str = actor_b.to_string();

    assert!(
        a_after.iter().any(|(actor, _)| actor == &actor_b_str),
        "A must have B's ops after sync"
    );
    assert!(
        b_after.iter().any(|(actor, _)| actor == &actor_a_str),
        "B must have A's ops after sync (regression: offline ops never delivered)"
    );
    assert_eq!(a_after.len(), 7, "A holds all 3+4 ops");
    assert_eq!(b_after.len(), 7, "B holds all 3+4 ops");
    assert_eq!(a_after, b_after, "both sides converged to the same op set");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_catchup() {
    // B is "up" the whole time. A writes ops while B isn't connected, then a
    // single A-initiated connection delivers them to B without B asking.
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // B starts with one op; A is empty for now.
    let b_nodes = seed_ops(dir_b.path(), actor_b, 1);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // Bring B up as a responder first (it's "online" and waiting).
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_responder(
        ep_b,
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    // Now A writes extra ops *while B is up but not yet connected to A*.
    let a_nodes = seed_ops(dir_a.path(), actor_a, 2);

    // A connects once and pushes. B never initiated.
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_ready_tx,
        ),
    )
    .await
    .expect("delta sync timed out")
    .expect("delta sync failed");

    // B must now have A's new ops, delivered on A's connection.
    let a_node_strs: BTreeSet<String> = a_nodes.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &a_node_strs)
        }),
        "B should have received A's offline ops on A's connection"
    );

    let b_after = created_nodes_on_disk(dir_b.path(), actor_b);
    let actor_a_str = actor_a.to_string();
    assert!(
        b_after.iter().any(|(actor, _)| actor == &actor_a_str),
        "B must hold A's ops after catch-up"
    );
    // B keeps its own op and gains A's two: 1 + 2 = 3.
    assert_eq!(
        b_after.len(),
        b_nodes.len() + a_nodes.len(),
        "B holds its own op plus A's two"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mesh_line_transitive_propagation() {
    // Three nodes in a LINE: A↔B and B↔C are linked, but A does NOT know C.
    // A must still converge to C's ops, carried transitively through B's op-log
    // (A↔B sync after B↔C sync hands A everything B knows, including C's ops).
    //
    // This is the existing transitive-propagation guarantee the mesh relies on,
    // proven over real QUIC on loopback (no relay, direct addrs).
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let dir_c = tempfile::tempdir().expect("C tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_c = ActorId::new();

    // Each node authors its own ops; the three logs start fully disjoint.
    let a_nodes = seed_ops(dir_a.path(), actor_a, 2);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 3);
    let c_nodes = seed_ops(dir_c.path(), actor_c, 4);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");
    let id_c = fresh_identity(dir_c.path(), "c");

    // B and C are responders (online, waiting). A and B will initiate.
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_responder(
        ep_b.clone(),
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    let (c_ready_tx, _c_ready_rx) = mpsc::channel::<()>();
    let ep_c = test_support::bind_sync_endpoint(&id_c)
        .await
        .expect("bind C endpoint");
    let c_addr = ep_c.addr();
    let _router_c = test_support::spawn_responder(
        ep_c,
        dir_c.path().to_path_buf(),
        shared_wid(),
        actor_c,
        c_ready_tx,
        &[id_b.node_id()],
    );

    // Link B↔C first: B (using its own endpoint as initiator) syncs with C, so
    // B's op-log now also holds C's ops.
    let (b_init_tx, _b_init_rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_b,
            c_addr,
            dir_b.path(),
            &shared_wid(),
            actor_b,
            b_init_tx,
        ),
    )
    .await
    .expect("B↔C sync timed out")
    .expect("B↔C sync failed");

    // B must now hold C's ops (the transitive carrier).
    let c_node_strs: BTreeSet<String> = c_nodes.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &c_node_strs)
        }),
        "B should hold C's ops after B↔C sync"
    );

    // Now link A↔B: A (which has NEVER contacted C) syncs with B. A should learn
    // BOTH B's own ops and C's ops, transitively.
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let (a_init_tx, _a_init_rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &shared_wid(),
            actor_a,
            a_init_tx,
        ),
    )
    .await
    .expect("A↔B sync timed out")
    .expect("A↔B sync failed");

    // A converges to the WHOLE mesh: its own + B's + C's ops, even though A never
    // dialed C.
    let all_nodes: BTreeSet<String> = a_nodes
        .iter()
        .chain(b_nodes.iter())
        .chain(c_nodes.iter())
        .map(|n| n.to_string())
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &all_nodes)
        }),
        "A should converge to C's ops transitively via B (A never knew C)"
    );

    let a_after = created_nodes_on_disk(dir_a.path(), actor_a);
    let actor_c_str = actor_c.to_string();
    assert!(
        a_after.iter().any(|(actor, _)| actor == &actor_c_str),
        "A must hold C's ops despite never connecting to C directly"
    );
    assert_eq!(
        a_after.len(),
        a_nodes.len() + b_nodes.len() + c_nodes.len(),
        "A holds the full mesh op set (A + B + C)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn membership_auto_discovery_then_direct_sync() {
    // Item 5: A pairs with B, and B already knows C. A must AUTO-DISCOVER C —
    // its peers.json gains C from B's membership broadcast — and then A syncs
    // C's ops DIRECTLY (no transitive hop through B).
    //
    // Drives the REAL membership code (`build_membership_payload` →
    // `parse_membership` → `merge_membership` via `test_support`), then proves
    // the catch-up loop dials the freshly-merged C directly. Membership + the
    // direct dial are the two halves of item 5's contract.
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let dir_c = tempfile::tempdir().expect("C tempdir");

    let actor_a = ActorId::new();
    let actor_c = ActorId::new();

    // C has ops A will eventually pull DIRECTLY once it discovers C.
    let c_nodes = seed_ops(dir_c.path(), actor_c, 5);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_c = fresh_identity(dir_c.path(), "c");
    let c_node_id = id_c.node_id().to_string();

    // C is a responder (online, waiting for a direct dial from A).
    let (c_ready_tx, _c_ready_rx) = mpsc::channel::<()>();
    let ep_c = test_support::bind_sync_endpoint(&id_c)
        .await
        .expect("bind C endpoint");
    let c_addr = ep_c.addr();
    let _router_c = test_support::spawn_responder(
        ep_c,
        dir_c.path().to_path_buf(),
        shared_wid(),
        actor_c,
        c_ready_tx,
        &[id_a.node_id()],
    );

    // B "knows" C: write C's full loopback EndpointAddr into B's peers.json,
    // exactly as B would have captured it at pairing time. This is the
    // reachability info B will gossip to A.
    let b_peers = dir_b.path().join("peers.json");
    {
        let mut b_store = PeersStore::load_or_default(&b_peers).expect("load B peers");
        let c_entry =
            outl_sync_iroh::PeerEntry::from_endpoint_addr(&c_addr, Some("device-c".into()))
                .expect("build C peer entry");
        b_store.add(c_entry).expect("B stores C");
    }

    // A pairs with B → A's peers.json gains B. (We model the post-pairing state
    // directly: the pairing roundtrip itself is covered by other tests; what
    // matters here is that A now receives B's membership broadcast.)
    let a_peers = dir_a.path().join("peers.json");
    let a_node_id = id_a.node_id().to_string();

    // ── Membership auto-discovery ────────────────────────────────────────────
    // B broadcasts its known peer list (containing C) over gossip; A receives it
    // and merges unknown peers. Before the merge, A doesn't know C.
    assert!(
        PeersStore::load_or_default(&a_peers)
            .expect("load A peers")
            .list()
            .iter()
            .all(|p| p.node_id != c_node_id),
        "A must NOT know C before membership gossip"
    );

    // Real round-trip: B builds the broadcast, A parses it.
    let gossiped = test_support::membership_roundtrip(&b_peers);
    assert!(
        gossiped.iter().any(|p| p.node_id == c_node_id),
        "B's membership broadcast should advertise C"
    );

    // A merges the gossiped list into its own peers.json (drops self, adds C).
    let added = test_support::membership_merge(&a_peers, &a_node_id, gossiped);
    assert_eq!(added, 1, "A should auto-discover exactly C");

    // A's peers.json now holds C — discovered without a manual A↔C pairing.
    let a_store = PeersStore::load_or_default(&a_peers).expect("reload A peers");
    assert!(
        a_store.list().iter().any(|p| p.node_id == c_node_id),
        "A's peers.json must gain C after membership gossip"
    );

    // ── Direct sync to the discovered peer ───────────────────────────────────
    // A's catch-up loop reloads peers.json each tick and dials C DIRECTLY. Drive
    // the real catch-up loop with a resolver reading A's peers.json (the same
    // thing production does), proving A pulls C's ops without a transitive hop.
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let a_peers_for_resolver = a_peers.clone();
    let resolver = move || match PeersStore::load_or_default(&a_peers_for_resolver) {
        Ok(store) => store
            .list()
            .iter()
            .filter_map(|p| p.iroh_endpoint_addr().ok())
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    let a_workspace = dir_a.path().to_path_buf();
    let catchup = tokio::spawn(async move {
        test_support::run_catch_up_loop(
            ep_a,
            Duration::from_millis(200),
            // No maintenance re-sync during the test; isolate the behavior under test.
            Duration::from_secs(3600),
            resolver,
            a_workspace,
            shared_wid(),
            actor_a,
            a_ready_tx,
            None,
        )
        .await;
    });

    // Within a couple of catch-up ticks, A holds C's full op history — pulled
    // DIRECTLY from C, the peer it auto-discovered.
    let c_node_strs: BTreeSet<String> = c_nodes.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &c_node_strs)
        }),
        "A should sync C's ops directly after auto-discovering C"
    );

    let a_after = created_nodes_on_disk(dir_a.path(), actor_a);
    let actor_c_str = actor_c.to_string();
    assert!(
        a_after.iter().any(|(actor, _)| actor == &actor_c_str),
        "A must hold C's ops after the direct catch-up dial"
    );
    assert_eq!(a_after.len(), c_nodes.len(), "A holds exactly C's 5 ops");

    catchup.abort();
}

/// Two devices at DIFFERENT local paths but with the SAME workspace id must
/// derive the SAME gossip topic — and a different id must derive a different one.
///
/// This is the core of the bug fix: the topic used to be `blake3(workspace_root)`,
/// so two real devices (desktop `~/outl-p2p`, mobile `…/outl`) computed different
/// topics and gossip never connected. Keying on the stable shared `WorkspaceId`
/// makes the topic path-independent.
#[test]
fn same_workspace_id_yields_same_topic_across_paths() {
    let id = WorkspaceId::from_raw("SHAREDWORKSPACE0000000000000");

    // Different local paths, same id → identical topic.
    let topic_desktop = test_support::topic_id_bytes(&id);
    let topic_mobile = test_support::topic_id_bytes(&id);
    assert_eq!(
        topic_desktop, topic_mobile,
        "same workspace id must produce the same gossip topic regardless of path"
    );

    // A genuinely different workspace → different topic.
    let other = WorkspaceId::from_raw("OTHERWORKSPACE00000000000000");
    assert_ne!(
        test_support::topic_id_bytes(&id),
        test_support::topic_id_bytes(&other),
        "distinct workspace ids must produce distinct topics"
    );
}

/// The serve side rejects a peer whose `SyncRequest.workspace_id` doesn't match
/// the local id: no ops cross between two genuinely-different workspaces.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delta_sync_rejects_mismatched_workspace_id() {
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    seed_ops(dir_a.path(), actor_a, 3);
    seed_ops(dir_b.path(), actor_b, 4);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // B's responder is on workspace "BBB"; A initiates with workspace "AAA".
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let b_addr = ep_b.addr();
    let _router_b = test_support::spawn_responder(
        ep_b,
        dir_b.path().to_path_buf(),
        WorkspaceId::from_raw("BBB00000000000000000000000000"),
        actor_b,
        b_ready_tx,
        &[id_a.node_id()],
    );

    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");

    // The initiator's stream may complete (the responder closes after sending
    // its reject frame), but NO ops must cross — B never learns A's ops and A
    // never learns B's.
    let _ = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_a,
            b_addr,
            dir_a.path(),
            &WorkspaceId::from_raw("AAA00000000000000000000000000"),
            actor_a,
            a_ready_tx,
        ),
    )
    .await;

    // Give any (erroneous) ingest a beat, then assert neither side gained the
    // other's ops.
    std::thread::sleep(Duration::from_millis(300));
    let a_after = created_nodes_on_disk(dir_a.path(), actor_a);
    let b_after = created_nodes_on_disk(dir_b.path(), actor_b);
    let actor_a_str = actor_a.to_string();
    let actor_b_str = actor_b.to_string();
    assert!(
        !a_after.iter().any(|(actor, _)| actor == &actor_b_str),
        "A must NOT receive B's ops across a workspace-id mismatch"
    );
    assert!(
        !b_after.iter().any(|(actor, _)| actor == &actor_a_str),
        "B must NOT receive A's ops across a workspace-id mismatch"
    );
}

/// Poll until a transport has published its pairing hub (i.e. `run_iroh` bound
/// the endpoint and registered the PAIRING_ALPN handler).
///
/// Probes with a deliberately invalid ticket: while the hub is absent the error
/// is "not started yet"; once the hub is live the same call fails on ticket
/// decode instead. No real dial happens (decode fails before connect).
async fn wait_for_pairing_ready(transport: &outl_sync_iroh::IrohSyncTransport) {
    let deadline = std::time::Instant::now() + STEP_TIMEOUT;
    loop {
        match transport.pair_join("not-a-real-ticket".into(), None).await {
            // Decode error => hub is live (we got past the "not started" gate).
            Err(e) if !e.to_string().contains("not started") => return,
            _ => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "transport never published its pairing hub"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Issue #159, end to end: an uninvited device that dials the armed host must
/// be refused **and must not consume the pairing window**.
///
/// This is the half that is easy to get wrong in the safe-looking direction.
/// Before the proof existed, `host_pairing` accepted exactly one connection —
/// so once a check that can *fail* was added, the first stranger to dial would
/// have ended the user's pairing session, and the device they were actually
/// trying to pair would find nothing listening. A security fix that turns into
/// a one-packet denial of the feature it protects is not a fix.
///
/// The attacker here knows the host's address but not the invite, which is the
/// exact position of any existing mesh member (membership gossip hands every
/// peer every other peer's address) or anyone who photographed the screen
/// rather than the code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_joiner_does_not_consume_the_pairing_window() {
    let host_dir = tempfile::tempdir().expect("host tempdir");
    let evil_dir = tempfile::tempdir().expect("attacker tempdir");
    let join_dir = tempfile::tempdir().expect("joiner tempdir");

    let host_identity = fresh_identity(host_dir.path(), "host");
    let evil_identity = fresh_identity(evil_dir.path(), "evil");
    let join_identity = fresh_identity(join_dir.path(), "join");

    let evil_node_id = evil_identity.node_id().to_string();
    let join_node_id = join_identity.node_id().to_string();

    let host_peers = host_dir.path().join("peers.json");
    let host_root = host_dir.path().to_path_buf();
    let host_peers_for_task = host_peers.clone();

    let (ticket_tx, ticket_rx) = tokio::sync::oneshot::channel::<String>();

    let host_task = tokio::spawn(async move {
        let mut ticket_tx = Some(ticket_tx);
        tokio::time::timeout(
            STEP_TIMEOUT,
            host_pairing(
                host_identity,
                &host_peers_for_task,
                &host_root,
                Some("host-device".into()),
                |ticket, _qr| {
                    if let Some(tx) = ticket_tx.take() {
                        let _ = tx.send(ticket.to_string());
                    }
                },
            ),
        )
        .await
        .expect("host pairing timed out — it did not stay armed after refusing")
        .expect("host pairing failed")
    });

    let advertised = ticket_rx.await.expect("never received host ticket");

    // Pin both dials to the host's loopback address. The host advertises one
    // direct address per local interface, and with no relay reachable iroh
    // 1.0.0's path selection settles on a dead one about half the time when the
    // same host is dialled twice. Leaving that in would make this test a
    // measurement of path selection rather than of the accept loop.
    let (advertised_addr, _) = decode_ticket(&advertised).expect("decode advertised ticket");
    let Some(loopback) = test_support::loopback_only(&advertised_addr) else {
        eprintln!("host bound no loopback address; skipping");
        return;
    };
    let real_ticket =
        test_support::retarget_ticket(&advertised, loopback).expect("retarget the ticket");

    // Forge a ticket for the SAME address with a secret the host never issued.
    // Everything the attacker needs is public; the only thing it lacks is the
    // invite, which is precisely what the proof tests.
    let (host_addr, _issued) = decode_ticket(&real_ticket).expect("decode the real ticket");
    let (forged_ticket, _forged_secret) = mint_ticket(&host_addr).expect("forge a ticket");
    // Same address, different secret: the attacker knows where the host is and
    // nothing else.
    assert_ne!(
        forged_ticket, real_ticket,
        "the forged ticket must differ from the issued one",
    );

    // 1. The attacker dials first and must be refused.
    let evil_peers = evil_dir.path().join("peers.json");
    let evil_result = tokio::time::timeout(
        STEP_TIMEOUT,
        join_pairing(
            evil_identity,
            &forged_ticket,
            &evil_peers,
            evil_dir.path(),
            Some("evil-device".into()),
        ),
    )
    .await
    .expect("attacker pairing attempt hung");
    assert!(
        evil_result.is_err(),
        "a device without the issued ticket must not pair",
    );

    // 2. The real device dials afterwards and must still get through — this is
    //    the re-arm. If the host had consumed its window on the refusal, this
    //    hangs until STEP_TIMEOUT and the assertion above the spawn fires.
    let join_peers = join_dir.path().join("peers.json");
    let join_outcome = tokio::time::timeout(
        STEP_TIMEOUT,
        join_pairing(
            join_identity,
            &real_ticket,
            &join_peers,
            join_dir.path(),
            Some("join-device".into()),
        ),
    )
    .await
    .expect("legitimate pairing timed out — the host stopped accepting after the refusal");
    let (join_entry, _adoption) = join_outcome.expect("legitimate pairing failed");

    let host_entry = host_task.await.expect("host task panicked");

    // 3. The host paired with the invited device, and only that one.
    assert_eq!(
        host_entry.node_id, join_node_id,
        "host must pair with the invited device",
    );
    let host_store = PeersStore::load_or_default(&host_peers).expect("load host peers");
    assert!(
        host_store.list().iter().any(|p| p.node_id == join_node_id),
        "the invited device is missing from the host's peers.json",
    );
    assert!(
        host_store.list().iter().all(|p| p.node_id != evil_node_id),
        "the refused device was persisted anyway — the check ran too late",
    );

    // 4. And the attacker learned nothing: no host entry on its side. The host
    //    verifies the proof BEFORE sending its payload, so a refused dialer
    //    never receives the workspace identity it was after.
    let evil_store = PeersStore::load_or_default(&evil_peers).expect("load attacker peers");
    assert!(
        evil_store.list().is_empty(),
        "a refused joiner must not come away with the host's peer entry",
    );
    assert!(
        !join_entry.node_id.is_empty(),
        "the joiner came away with the host's entry",
    );
}
