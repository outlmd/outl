//! Revocation actually revokes: every protocol on the endpoint, both directions.
//!
//! `outl peer remove` drops a device from `peers.json`, and issue #158 made
//! `SYNC_ALPN` refuse a dialer that is not in that file. It stopped there, and
//! "there" was one of three content protocols on one endpoint:
//!
//! - `SNAPSHOT_ALPN` shipped `snap-<actor>.bin` — the **materialized
//!   workspace**, every page — to any dialer that spoke the ALPN.
//! - `ASSET_ALPN` shipped the `assets/` manifest and then every blob in it.
//! - The **initiator** authorized nobody, so an announce on the gossip topic
//!   (whose id a revoked device already knows) made this device dial the
//!   announcer and ingest its ops.
//!
//! So a removed device kept full read access to the graph and to every uploaded
//! file, and could still get ops in. Only its *pushes* were refused, which is
//! the half a user is least likely to test.
//!
//! Every test here is a DENY unless its name says otherwise. The allow cases are
//! here too and deliberately outnumbered: a check that refuses everything passes
//! every deny test ever written, so the allows are what stop the fix from
//! becoming an outage.
//!
//! The sibling question — how a node id gets INTO the `peers.json` every check
//! here reads, including the open gossip hole pinned by an `#[ignore]`d test —
//! lives in `tests/membership_trust.rs`.

use std::path::Path;
use std::sync::mpsc;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_sync_iroh::{test_support, workspace_peers_path, PeersStore};

// `common/` is shared by six suites; a helper another suite needs is dead code
// here, and that is not a defect in either file.
#[allow(dead_code)]
mod common;

use common::{fresh_identity, now_ms, wait_until, STEP_TIMEOUT};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Write a decodable `snap-<actor>.bin` into `<root>/.outl/snapshots/`, so a
/// disclosure test has something real to fail to steal.
fn seed_snapshot(workspace_root: &Path, actor: ActorId) -> Vec<u8> {
    use std::collections::{BTreeMap, BTreeSet};

    let dir = workspace_root.join(".outl").join("snapshots");
    let node = NodeId::new();
    let mut nodes = BTreeMap::new();
    nodes.insert(node, (NodeId::root(), Fractional::first()));
    let mut cutoff = BTreeMap::new();
    cutoff.insert(actor, Hlc::new(now_ms(), 0, actor));
    let mut block_text = BTreeMap::new();
    block_text.insert(node, "a private page nobody revoked may read".to_string());
    let body = outl_core::SnapshotBody::from_parts(
        actor,
        cutoff,
        nodes,
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        block_text,
    )
    .expect("build test snapshot body");
    outl_core::snapshot::write_to_disk(&dir, &body).expect("write snapshot");
    std::fs::read(dir.join(format!("snap-{actor}.bin"))).expect("read back snapshot")
}

/// `outl peer remove` against `workspace_root`'s peer list, tombstone included.
fn revoke(workspace_root: &Path, peer: iroh::EndpointId) {
    let path = workspace_peers_path(workspace_root);
    let mut store = PeersStore::load_or_default(&path).expect("load peers");
    assert!(
        store.remove(&peer.to_string()).expect("remove peer"),
        "the peer must have been listed before it can be revoked"
    );
}

/// Pair `peer` into `workspace_root`'s peer list, then revoke it.
fn pair_then_revoke(workspace_root: &Path, peer: iroh::EndpointId) {
    test_support::authorize_peer(workspace_root, peer);
    revoke(workspace_root, peer);
}

/// Bind the host endpoint and mount the production snapshot responder on it,
/// with `authorized_peers` seeded into its `peers.json`. The returned router
/// must stay alive for as long as the host must answer.
async fn snapshot_host(
    identity: &outl_sync_iroh::IrohIdentity,
    workspace_root: &Path,
    actor: ActorId,
    authorized_peers: &[iroh::EndpointId],
) -> (iroh::EndpointAddr, iroh::protocol::Router) {
    let endpoint = test_support::bind_sync_endpoint(identity)
        .await
        .expect("bind host endpoint");
    let addr = endpoint.addr();
    let router = test_support::spawn_snapshot_responder(
        endpoint,
        workspace_root.to_path_buf(),
        actor,
        authorized_peers,
    );
    (addr, router)
}

/// Does the joiner hold any cached peer snapshot at all?
fn holds_any_snapshot(workspace_root: &Path) -> bool {
    let dir = workspace_root.join(".outl").join("snapshots");
    std::fs::read_dir(&dir)
        .map(|mut it| it.any(|e| e.is_ok()))
        .unwrap_or(false)
}

/// Does the joiner hold any asset at all?
fn holds_any_asset(workspace_root: &Path) -> bool {
    std::fs::read_dir(workspace_root.join("assets"))
        .map(|mut it| it.any(|e| e.is_ok()))
        .unwrap_or(false)
}

// ─────────────────────────────────────────────────────────────────────────────
// DENY — SNAPSHOT_ALPN (the materialized workspace)
// ─────────────────────────────────────────────────────────────────────────────

/// DENY. A device the user removed must not be able to pull the whole graph.
///
/// This is the sharpest half of the bug: `outl peer remove` told the user the
/// device was gone while the device kept a complete, current copy of every page
/// one ALPN over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_peer_gets_no_snapshot() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_evil = tempfile::tempdir().expect("evil tempdir");
    let host_actor = ActorId::new();
    let host_bytes = seed_snapshot(dir_host.path(), host_actor);
    assert!(!host_bytes.is_empty(), "the host must hold a real snapshot");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_evil = fresh_identity(dir_evil.path(), "evil");

    // The attacker WAS paired, then was removed.
    pair_then_revoke(dir_host.path(), id_evil.node_id());

    let (host_addr, _router) = snapshot_host(&id_host, dir_host.path(), host_actor, &[]).await;

    let ep_evil = test_support::bind_sync_endpoint(&id_evil)
        .await
        .expect("bind evil endpoint");
    let (tx, _rx) = mpsc::channel::<()>();
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_evil, host_addr, dir_evil.path(), tx),
    )
    .await
    .expect("snapshot pull did not finish within the step timeout");

    assert!(
        result.is_err(),
        "a revoked device must be refused, not served an empty frame it could \
         retry past: got {result:?}"
    );
    assert!(
        !holds_any_snapshot(dir_evil.path()),
        "the revoked device wrote a snapshot to disk — it read the workspace"
    );
}

/// DENY. A device that was never paired at all — the plain stranger case.
///
/// Separate from revocation on purpose: a fix that only honoured tombstones
/// would pass the test above and still serve the whole graph to anyone who
/// dialled, because a stranger has no tombstone to honour.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_dialer_gets_no_snapshot() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_evil = tempfile::tempdir().expect("evil tempdir");
    let host_actor = ActorId::new();
    seed_snapshot(dir_host.path(), host_actor);

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_evil = fresh_identity(dir_evil.path(), "evil");
    // The host has a peer list with a DIFFERENT device in it, so the refusal
    // cannot be "the list was empty".
    let other = fresh_identity(dir_host.path(), "other").node_id();
    let (host_addr, _router) = snapshot_host(&id_host, dir_host.path(), host_actor, &[other]).await;

    let ep_evil = test_support::bind_sync_endpoint(&id_evil)
        .await
        .expect("bind evil endpoint");
    let (tx, _rx) = mpsc::channel::<()>();
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_evil, host_addr, dir_evil.path(), tx),
    )
    .await
    .expect("snapshot pull did not finish within the step timeout");

    assert!(result.is_err(), "an unpaired stranger must be refused");
    assert!(!holds_any_snapshot(dir_evil.path()));
}

/// DENY. A `peers.json` we cannot parse is a refusal, never a fall-back to open
/// access.
///
/// The credential store is the only thing standing between a dialer and the
/// workspace here, so "we could not read it" has exactly one safe reading. A
/// version that logged and served would turn one corrupt file into total
/// disclosure, and would do it quietly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_malformed_peer_list_denies_the_snapshot() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_peer = tempfile::tempdir().expect("peer tempdir");
    let host_actor = ActorId::new();
    seed_snapshot(dir_host.path(), host_actor);

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_peer = fresh_identity(dir_peer.path(), "peer");
    // Genuinely paired — and then the file is damaged.
    test_support::authorize_peer(dir_host.path(), id_peer.node_id());
    let path = workspace_peers_path(dir_host.path());
    std::fs::write(&path, b"{\"peers\": [ truncated").expect("damage peers.json");

    let (host_addr, _router) = snapshot_host(&id_host, dir_host.path(), host_actor, &[]).await;

    let ep_peer = test_support::bind_sync_endpoint(&id_peer)
        .await
        .expect("bind peer endpoint");
    let (tx, _rx) = mpsc::channel::<()>();
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_peer, host_addr, dir_peer.path(), tx),
    )
    .await
    .expect("snapshot pull did not finish within the step timeout");

    assert!(
        result.is_err(),
        "an unreadable peer list must fail CLOSED — it cannot prove an approval"
    );
    assert!(!holds_any_snapshot(dir_peer.path()));
}

/// DENY. No `peers.json` at all — a fresh workspace nobody has paired into.
///
/// The absent-credential mirror of the malformed case. `load_or_default`
/// returns an EMPTY store here rather than an error, so this is the one path
/// where the refusal comes from the emptiness and not from a read failure;
/// a guard written only against the parse error would wave this through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_absent_peer_list_denies_the_snapshot() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_evil = tempfile::tempdir().expect("evil tempdir");
    let host_actor = ActorId::new();
    seed_snapshot(dir_host.path(), host_actor);
    assert!(
        !workspace_peers_path(dir_host.path()).exists(),
        "this test is only meaningful with no peers.json on disk"
    );

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_evil = fresh_identity(dir_evil.path(), "evil");

    let (host_addr, _router) = snapshot_host(&id_host, dir_host.path(), host_actor, &[]).await;

    let ep_evil = test_support::bind_sync_endpoint(&id_evil)
        .await
        .expect("bind evil endpoint");
    let (tx, _rx) = mpsc::channel::<()>();
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_evil, host_addr, dir_evil.path(), tx),
    )
    .await
    .expect("snapshot pull did not finish within the step timeout");

    assert!(result.is_err(), "an unpaired workspace serves nobody");
    assert!(!holds_any_snapshot(dir_evil.path()));
}

// ─────────────────────────────────────────────────────────────────────────────
// DENY — ASSET_ALPN (every uploaded file)
// ─────────────────────────────────────────────────────────────────────────────

/// DENY. A revoked device must not keep pulling the workspace's uploaded files.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_peer_gets_no_assets() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_evil = tempfile::tempdir().expect("evil tempdir");
    let secret = b"a private PDF the user uploaded";
    test_support::write_test_asset(dir_host.path(), secret, "pdf");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_evil = fresh_identity(dir_evil.path(), "evil");
    pair_then_revoke(dir_host.path(), id_evil.node_id());

    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router = test_support::spawn_asset_responder(ep_host, dir_host.path(), &[]);

    let ep_evil = test_support::bind_sync_endpoint(&id_evil)
        .await
        .expect("bind evil endpoint");
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_asset_pull(&ep_evil, host_addr, dir_evil.path()),
    )
    .await
    .expect("asset pull did not finish within the step timeout");

    assert!(
        result.is_err(),
        "a revoked device must be refused: {result:?}"
    );
    assert!(
        !holds_any_asset(dir_evil.path()),
        "the revoked device pulled a file out of the workspace"
    );
}

/// The two assets a staged pull asks for, in request order.
const FIRST_ASSET: &[u8] = b"the upload the peer was still entitled to";
const SECOND_ASSET: &[u8] = b"the upload it must not get after removal";

/// Pull two assets over ONE connection, optionally running `outl peer remove`
/// between the blobs. Returns the bodies the host answered with — SHORT when it
/// hung up, full-with-an-empty-entry when it kept serving and merely lacked the
/// file. Only the first is a refusal, and a test blind to the difference would
/// pass against a responder that leaks every blob.
async fn staged_two_asset_pull(revoke_midway: bool) -> Vec<Vec<u8>> {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_peer = tempfile::tempdir().expect("peer tempdir");
    let names = vec![
        test_support::write_test_asset(dir_host.path(), FIRST_ASSET, "pdf"),
        test_support::write_test_asset(dir_host.path(), SECOND_ASSET, "pdf"),
    ];

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_peer = fresh_identity(dir_peer.path(), "peer");
    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router =
        test_support::spawn_asset_responder(ep_host, dir_host.path(), &[id_peer.node_id()]);

    let ep_peer = test_support::bind_sync_endpoint(&id_peer)
        .await
        .expect("bind peer endpoint");
    let host_root = dir_host.path().to_path_buf();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_staged_asset_pull(&ep_peer, host_addr, &names, |i| {
            if i == 0 && revoke_midway {
                revoke(&host_root, id_peer.node_id());
            }
        }),
    )
    .await
    .expect("staged asset pull did not finish within the step timeout")
    .expect("the staged pull itself must reach the host")
}

/// DENY. Revocation has to land on a transfer already in flight.
///
/// The asset handler authorized once per connection and then served one blob
/// per request, unbounded and initiator-paced, so a peer that connected while
/// approved kept draining `assets/` for as long as it kept asking. `SYNC_ALPN`
/// never had this shape — its check runs per exchange on a pooled connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_revoked_mid_connection_gets_no_further_assets() {
    let bodies = staged_two_asset_pull(true).await;
    assert_eq!(
        bodies.len(),
        1,
        "the host kept answering a device the user had just removed: {bodies:?}"
    );
    // Real bytes, or a handler that refuses everything would pass this too.
    assert_eq!(bodies[0], FIRST_ASSET);
}

/// ALLOW. Re-checking per request must not cost an approved peer its transfer —
/// the mirror of the deny above, and what fails if the re-check refuses on
/// anything but a genuine removal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approved_peer_gets_every_asset_on_one_connection() {
    let bodies = staged_two_asset_pull(false).await;
    assert_eq!(
        bodies.len(),
        2,
        "an approved peer must be served both blobs"
    );
    assert_eq!(bodies[0], FIRST_ASSET);
    assert_eq!(bodies[1], SECOND_ASSET);
}

/// DENY. The MANIFEST is itself a disclosure, so the refusal has to land before
/// it — the names are content hashes and their count is the size of the user's
/// upload history.
///
/// Asserting only "no bytes transferred" would pass a version that shipped the
/// manifest and refused the blobs, which still leaks. The pull errors on the
/// manifest read, so an `Ok(0)` here is the failure signature to watch for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_dialer_gets_no_asset_manifest() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_evil = tempfile::tempdir().expect("evil tempdir");
    test_support::write_test_asset(dir_host.path(), b"one", "png");
    test_support::write_test_asset(dir_host.path(), b"two", "png");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_evil = fresh_identity(dir_evil.path(), "evil");

    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router = test_support::spawn_asset_responder(ep_host, dir_host.path(), &[]);

    let ep_evil = test_support::bind_sync_endpoint(&id_evil)
        .await
        .expect("bind evil endpoint");
    let result = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_asset_pull(&ep_evil, host_addr, dir_evil.path()),
    )
    .await
    .expect("asset pull did not finish within the step timeout");

    assert!(
        result.is_err(),
        "the refusal must land BEFORE the manifest, so the dialer learns \
         neither the hashes nor how many assets exist: got {result:?}"
    );
    assert!(!holds_any_asset(dir_evil.path()));
}

/// DENY. A snapshot with no per-actor `cutoff` must never be cached.
///
/// Not a shape any honest peer can produce: `build_snapshot_body` returns
/// `None` rather than an empty cutoff, so a body carrying one came from a
/// forged or corrupt source — which is reachable precisely because
/// `SNAPSHOT_ALPN` accepts a body chosen by the peer.
///
/// What it buys the attacker is a **skipped guard**, not just a bad cache.
/// `outl_core`'s adoption check reads `if let Some(max) = body.cutoff.values()
/// .max()`, so an empty cutoff makes it not run at all — and every actor absent
/// from the cutoff means "I have seen none of your ops", so the victim's entire
/// log replays on top of the attacker's `nodes` / `block_text`. That is the
/// divergence the guard exists to prevent, and since RFC 0263 it also re-arms
/// the duplicate-`Create` defect that RFC fixed.
///
/// `outl-core` owns the authoritative refusal (a snapshot also arrives by file
/// transport and by restored backup, paths the puller never sees). This pins
/// the transport half: the poisoned file is never written in the first place.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_snapshot_with_no_cutoff_is_never_cached() {
    use std::collections::{BTreeMap, BTreeSet};

    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_peer = tempfile::tempdir().expect("peer tempdir");
    let host_actor = ActorId::new();

    // A decodable, hash-valid body with an EMPTY cutoff — what a malicious
    // peer serves, and what no `build_snapshot_body` ever produces.
    let node = NodeId::new();
    let mut nodes = BTreeMap::new();
    nodes.insert(node, (NodeId::root(), Fractional::first()));
    let mut block_text = BTreeMap::new();
    block_text.insert(node, "content the attacker chose".to_string());
    let body = outl_core::SnapshotBody::from_parts(
        host_actor,
        BTreeMap::new(),
        nodes,
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        block_text,
    )
    .expect("build a cutoff-less body");
    let snap_dir = dir_host.path().join(".outl").join("snapshots");
    outl_core::snapshot::write_to_disk(&snap_dir, &body).expect("write");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_peer = fresh_identity(dir_peer.path(), "peer");
    // Fully authorized: this refusal is about the CONTENT, not the dialer, and
    // an authorized-peer test is the only one that proves that.
    let (host_addr, _router) =
        snapshot_host(&id_host, dir_host.path(), host_actor, &[id_peer.node_id()]).await;

    let ep_peer = test_support::bind_sync_endpoint(&id_peer)
        .await
        .expect("bind peer endpoint");
    let (tx, _rx) = mpsc::channel::<()>();
    let wrote = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_peer, host_addr, dir_peer.path(), tx),
    )
    .await
    .expect("snapshot pull timed out")
    .expect("a refused body is skipped, not an error — the pull is best-effort");

    assert!(
        !wrote,
        "a cutoff-less snapshot must be skipped, not adopted"
    );
    assert!(
        !holds_any_snapshot(dir_peer.path()),
        "the poisoned snapshot landed on disk — boot would adopt it with the \
         convergence guard skipped"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ALLOW — the fix must not become an outage
// ─────────────────────────────────────────────────────────────────────────────

/// ALLOW. A device that IS paired still pulls the snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approved_peer_still_pulls_the_snapshot() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_peer = tempfile::tempdir().expect("peer tempdir");
    let host_actor = ActorId::new();
    let host_bytes = seed_snapshot(dir_host.path(), host_actor);

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_peer = fresh_identity(dir_peer.path(), "peer");
    let (host_addr, _router) =
        snapshot_host(&id_host, dir_host.path(), host_actor, &[id_peer.node_id()]).await;

    let ep_peer = test_support::bind_sync_endpoint(&id_peer)
        .await
        .expect("bind peer endpoint");
    let (tx, _rx) = mpsc::channel::<()>();
    let wrote = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_snapshot_pull(&ep_peer, host_addr, dir_peer.path(), tx),
    )
    .await
    .expect("snapshot pull timed out")
    .expect("an approved peer must still be served");
    assert!(wrote, "the approved peer must receive the snapshot");

    let landed = dir_peer
        .path()
        .join(".outl")
        .join("snapshots")
        .join(format!("snap-{host_actor}.bin"));
    assert!(wait_until(STEP_TIMEOUT, || landed.exists()));
    assert_eq!(
        std::fs::read(&landed).expect("read"),
        host_bytes,
        "the served snapshot must be byte-identical"
    );
}

/// ALLOW. A device that IS paired still pulls assets.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approved_peer_still_pulls_assets() {
    let dir_host = tempfile::tempdir().expect("host tempdir");
    let dir_peer = tempfile::tempdir().expect("peer tempdir");
    let bytes = b"a file the paired device is entitled to";
    let name = test_support::write_test_asset(dir_host.path(), bytes, "pdf");

    let id_host = fresh_identity(dir_host.path(), "host");
    let id_peer = fresh_identity(dir_peer.path(), "peer");

    let ep_host = test_support::bind_sync_endpoint(&id_host)
        .await
        .expect("bind host endpoint");
    let host_addr = ep_host.addr();
    let _router =
        test_support::spawn_asset_responder(ep_host, dir_host.path(), &[id_peer.node_id()]);

    let ep_peer = test_support::bind_sync_endpoint(&id_peer)
        .await
        .expect("bind peer endpoint");
    let written = tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_asset_pull(&ep_peer, host_addr, dir_peer.path()),
    )
    .await
    .expect("asset pull timed out")
    .expect("an approved peer must still be served");

    assert_eq!(written, 1);
    assert_eq!(
        std::fs::read(dir_peer.path().join("assets").join(&name)).expect("read"),
        bytes,
        "the served asset must be byte-identical"
    );
}
