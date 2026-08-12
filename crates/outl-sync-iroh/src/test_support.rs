//! Test-only hooks that drive the **real** sync code paths (no logic copy).
//!
//! Integration tests live in `tests/` — a separate crate — so they can only
//! reach `pub` items. These thin wrappers expose the production `delta_sync`
//! initiator and the production `SyncProtocolHandler` responder (mounted on a
//! `Router`) so a test can stand up two endpoints over loopback and reconcile
//! them through the exact same wire code the transport runs.
//!
//! Connecting via a full [`iroh::EndpointAddr`] (with the direct addrs from
//! `endpoint.addr()`) keeps loopback sync deterministic without a relay or n0
//! discovery — the transport's own `delta_sync` connects by bare node id, which
//! would otherwise depend on discovery being reachable.
//!
//! This module is `#[doc(hidden)]` and exists purely so the out-of-crate
//! integration tests can exercise the real reconciliation logic.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::protocol::Router;
use outl_core::id::ActorId;
use outl_core::WorkspaceId;

use crate::engine::{delta_sync, SyncProtocolHandler};
use crate::engine_catchup::run_catch_up;
use crate::peers::{workspace_peers_path, PeerEntry, PeersStore};
use crate::protocol::SYNC_ALPN;

/// Authorize `peer` as an approved device for the responder rooted at
/// `workspace_root`, by writing a minimal [`PeerEntry`] into its
/// `<root>/.outl/peers.json` — the same file real pairing writes.
///
/// The production `SyncProtocolHandler::serve` rejects any connection whose
/// `remote_id()` is not in `peers.json` (issue #158). The loopback tests stand
/// up two endpoints and sync them as a *paired* workspace, so each responder
/// must know the initiator(s) it's expected to serve. Call this for every
/// initiator node id before (or after) `spawn_responder`; a node id absent from
/// this list is treated as unknown/revoked and refused, which is exactly what
/// the security check tests assert.
pub fn authorize_peer(workspace_root: &Path, peer: iroh::EndpointId) {
    let path = workspace_peers_path(workspace_root);
    let mut store = PeersStore::load_or_default(&path).expect("load peers store");
    store
        .add(PeerEntry {
            node_id: peer.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .expect("authorize peer in peers.json");
}

/// Bind a sync-ALPN endpoint with the given identity, exactly like the
/// transport's `run_iroh` does.
pub async fn bind_sync_endpoint(identity: &crate::IrohIdentity) -> Result<iroh::Endpoint> {
    // STOPGAP: IPv4-only bind, matching the production endpoints (engine /
    // pairing / status) so the loopback tests exercise the same code path.
    // IPv4-only also makes loopback sync more deterministic: `endpoint.addr()`
    // carries only the 127.0.0.1 direct addr, so the test connect never races a
    // `[::1]` path. Revert when iroh > 1.0.0 ships the multipath fallback fix.
    // See `crate::bind`.
    crate::bind::n0_builder_ipv4_only(None)
        .secret_key(identity.secret_key().clone())
        .alpns(vec![SYNC_ALPN.to_vec()])
        .bind()
        .await
        .context("bind sync endpoint")
}

/// A responder that completes the sync exchange up to — but NOT
/// including — the durable-ingest `close(0, "done")`.
///
/// It sends a valid (empty) response + empty push, drains the
/// initiator's frames so its writes all succeed, then closes with a
/// **non-"done"** code WITHOUT ingesting. This simulates a
/// suspended-iPhone / carrier-NAT-drop peer: the connection completes
/// cleanly for the initiator, but the peer never durably persisted the
/// push. Used to prove `delta_sync` does NOT report success in that
/// case (the false-"catch-up: sync ok" bug).
#[derive(Debug, Clone)]
struct HalfResponder;

impl iroh::protocol::ProtocolHandler for HalfResponder {
    async fn accept(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> std::result::Result<(), iroh::protocol::AcceptError> {
        let Ok((mut send, mut recv)) = conn.accept_bi().await else {
            return Ok(());
        };
        // Send a valid response + empty push so the initiator proceeds
        // all the way through pushing its own ops.
        let response = crate::protocol::SyncResponse {
            vector_clock: std::collections::HashMap::new(),
        };
        if let Ok(bytes) = crate::protocol::encode_response(&response) {
            let _ = send.write_all(&bytes).await;
        }
        if let Ok(bytes) = crate::protocol::encode_ops_blob(&[]) {
            let _ = send.write_all(&bytes).await;
        }
        let _ = send.finish();
        // Drain the initiator's request + push so ITS writes succeed
        // (the connection looks healthy from the initiator's side) — but
        // never ingest, and close with a code that is NOT the "done"
        // durable-ingest sentinel.
        let _ = recv.read_to_end(16 * 1024 * 1024).await;
        conn.close(9u32.into(), b"early-no-ingest");
        Ok(())
    }
}

/// Mount a `HalfResponder` (completes the exchange but never confirms
/// durable ingest) on a `Router`. See the `HalfResponder` doc.
pub fn spawn_half_responder(endpoint: iroh::Endpoint) -> Router {
    Router::builder(endpoint)
        .accept(SYNC_ALPN, HalfResponder)
        .spawn()
}

/// Mount the production `SyncProtocolHandler` on a `Router` and return it.
///
/// `authorized_peers` is seeded into the responder's `peers.json` before the
/// handler goes up, mirroring a real paired relationship: the production serve
/// side refuses any connection whose `remote_id()` is not an approved peer
/// (issue #158). Pass every initiator node id the responder is expected to
/// serve; pass an empty slice to stand up a responder that trusts nobody (used
/// by the revocation regression test to prove an unknown peer is rejected).
///
/// Keep the returned `Router` alive for as long as the responder must accept
/// connections; drop it (or call `shutdown()`) to stop serving.
pub fn spawn_responder(
    endpoint: iroh::Endpoint,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    authorized_peers: &[iroh::EndpointId],
) -> Router {
    for peer in authorized_peers {
        authorize_peer(&workspace_root, *peer);
    }
    Router::builder(endpoint)
        .accept(
            SYNC_ALPN,
            SyncProtocolHandler {
                workspace_root,
                workspace_id: Arc::new(RwLock::new(workspace_id)),
                actor,
                peer_ready_tx,
                // A fresh per-responder append guard: the test responder is the
                // only writer to its workspace, so a standalone lock is enough.
                append_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                inbound_serves: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            },
        )
        .spawn()
}

/// Mount the production `SnapshotProtocolHandler` on a `Router` and return it.
///
/// The responder serves `<workspace_root>/.outl/snapshots/snap-<actor>.bin` to
/// any dialer on [`crate::SNAPSHOT_ALPN`] — an empty frame when it has no
/// snapshot. Keep the returned `Router` alive for as long as it must accept
/// connections. Lets a loopback test exercise the real Phase-2 snapshot transfer
/// (server side) over real QUIC.
pub fn spawn_snapshot_responder(
    endpoint: iroh::Endpoint,
    workspace_root: PathBuf,
    actor: ActorId,
) -> Router {
    Router::builder(endpoint)
        .accept(
            crate::protocol::SNAPSHOT_ALPN,
            crate::engine_snapshot::SnapshotProtocolHandler {
                workspace_root,
                actor,
            },
        )
        .spawn()
}

/// Mount BOTH the sync and snapshot responders on ONE `Router` — the exact
/// shape `run_iroh` stands up (one endpoint per identity, several ALPNs). Lets a
/// loopback test prove op-sync still works while the snapshot ALPN is also
/// accepted on the same endpoint.
pub fn spawn_sync_and_snapshot_responder(
    endpoint: iroh::Endpoint,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    authorized_peers: &[iroh::EndpointId],
) -> Router {
    for peer in authorized_peers {
        authorize_peer(&workspace_root, *peer);
    }
    Router::builder(endpoint)
        .accept(
            SYNC_ALPN,
            SyncProtocolHandler {
                workspace_root: workspace_root.clone(),
                workspace_id: Arc::new(RwLock::new(workspace_id)),
                actor,
                peer_ready_tx,
                append_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                inbound_serves: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            },
        )
        .accept(
            crate::protocol::SNAPSHOT_ALPN,
            crate::engine_snapshot::SnapshotProtocolHandler {
                workspace_root,
                actor,
            },
        )
        .spawn()
}

/// Run the production snapshot pull (initiator side) against `peer` — the exact
/// call `drain_pair_completions` makes after the immediate delta-sync.
///
/// Dials `peer` on [`crate::SNAPSHOT_ALPN`], reads the snapshot frame, and (when
/// non-empty + decodable) writes it to
/// `<workspace_root>/.outl/snapshots/snap-<peer-actor>.bin`, firing
/// `peer_ready_tx`. Returns `true` when a snapshot was written, `false` when the
/// peer had none.
pub async fn run_snapshot_pull(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
) -> Result<bool> {
    crate::engine_snapshot::pull_snapshot_from_peer(
        endpoint,
        peer.into(),
        workspace_root,
        &peer_ready_tx,
        &crate::progress::ProgressSink::default(),
    )
    .await
}

/// Mount the production `AssetProtocolHandler` on a `Router` and return it.
///
/// The responder serves `<workspace_root>/assets/` (the manifest + per-name
/// bytes) to any dialer on [`crate::ASSET_ALPN`] — an empty manifest when the
/// dir is absent. Keep the returned `Router` alive for as long as it must accept
/// connections. Lets a loopback test exercise the real binary-asset transfer
/// (server side) over real QUIC.
pub fn spawn_asset_responder(endpoint: iroh::Endpoint, workspace_root: PathBuf) -> Router {
    Router::builder(endpoint)
        .accept(
            crate::protocol::ASSET_ALPN,
            crate::engine_assets::AssetProtocolHandler { workspace_root },
        )
        .spawn()
}

/// Write a content-addressed asset (`<root>/assets/<hash>.<ext>`) exactly like
/// `outl_actions::import_asset` would, and return its basename.
///
/// Lets a loopback test seed a peer's `assets/` without pulling `outl-actions` /
/// `outl-md` into the test crate's own dependency set — the filename IS the
/// sha-256 of `bytes`, so the puller's content-hash check passes.
pub fn write_test_asset(workspace_root: &Path, bytes: &[u8], ext: &str) -> String {
    let dir = outl_actions::assets_dir(workspace_root);
    std::fs::create_dir_all(&dir).expect("create assets dir");
    let name = format!("{}.{ext}", outl_md::asset::hash_bytes(bytes));
    std::fs::write(dir.join(&name), bytes).expect("write test asset");
    name
}

/// Run the production asset pull (initiator side) against `peer` — the exact call
/// `drain_pair_completions` / the catch-up loop make after the delta-sync.
///
/// Dials `peer` on [`crate::ASSET_ALPN`], negotiates the manifest, and writes
/// every asset the peer holds that `workspace_root/assets/` lacks (atomically,
/// content-hash-verified). Returns how many assets were written.
pub async fn run_asset_pull(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
) -> Result<usize> {
    crate::engine_assets::pull_assets_from_peer(
        endpoint,
        peer.into(),
        workspace_root,
        &crate::progress::ProgressSink::default(),
    )
    .await
}

/// [`run_delta_sync`] with a live progress sink, so a test can assert what the
/// user is TOLD, not only whether the sync converged.
///
/// The two are different questions and the repo only ever tested the first.
/// `SyncProgress` is cosmetic by design (a dropped update never breaks a sync),
/// which is exactly why nothing else catches a wrong one: a pass classified as
/// the wrong colour still errors, still re-pushes, still converges. The only
/// symptom is on screen.
/// The one body every `run_delta_sync*` helper shares: a fresh append lock,
/// then the production initiator.
///
/// The three public forms differ only in where the connection pool comes from
/// and whether progress is captured, so they are one-liners over this.
async fn drive_delta_sync(
    conns: &crate::peer_conn::PeerConnections,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    progress: crate::progress::ProgressSink,
) -> Result<()> {
    let append_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    delta_sync(
        conns,
        peer,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        &append_lock,
        &progress,
    )
    .await
}

pub async fn run_delta_sync_with_progress(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    progress_tx: std::sync::mpsc::Sender<outl_actions::SyncProgress>,
) -> Result<()> {
    drive_delta_sync(
        &crate::peer_conn::PeerConnections::new(endpoint.clone()),
        peer,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        crate::progress::ProgressSink::new(progress_tx),
    )
    .await
}

/// Run the production `delta_sync` initiator against `peer` (a full
/// [`iroh::EndpointAddr`] from the responder's `endpoint.addr()`).
pub async fn run_delta_sync(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
) -> Result<()> {
    drive_delta_sync(
        &crate::peer_conn::PeerConnections::new(endpoint.clone()),
        peer,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        crate::progress::ProgressSink::default(),
    )
    .await
}

/// Drive the production catch-up loop with a test-controlled tick `period` and a
/// `resolve_peers` closure (called once per tick) that yields the peers to dial.
///
/// This is the exact engine `run_iroh` spawns at boot; the only difference is
/// that production's resolver reloads `peers.json` and builds an addr from each
/// [`crate::PeerEntry`] (id + relay), whereas a test injects loopback
/// [`iroh::EndpointAddr`]s with direct addrs so no relay is needed. Lets a test
/// prove "a peer added AFTER the loop started gets caught up" over real QUIC.
///
/// Runs until the spawned task is dropped/aborted (the loop never returns).
///
/// `wid_changed`: `Some(rx)` wires the workspace-id-change broadcast so a test
/// can prove the loop **clears its per-session `synced` dedup and re-dials**
/// when the joiner adopts the host's id at runtime; `None` drives a fixed id and
/// asserts plain convergence (the adoption path is covered by
/// `catch_up_redials_after_workspace_id_change`).
#[allow(clippy::too_many_arguments)]
pub async fn run_catch_up_loop<F>(
    endpoint: iroh::Endpoint,
    period: Duration,
    resync_after: Duration,
    resolve_peers: F,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    wid_changed: Option<tokio::sync::broadcast::Receiver<WorkspaceId>>,
) where
    F: FnMut() -> Vec<iroh::EndpointAddr>,
{
    run_catch_up(
        crate::peer_conn::PeerConnections::new(endpoint.clone()),
        period,
        resync_after,
        resolve_peers,
        workspace_root,
        Arc::new(RwLock::new(workspace_id)),
        actor,
        peer_ready_tx,
        // Tests assert sync convergence, not the GUI reachability projection,
        // so they don't record health.
        None,
        // Fresh append guard; no shared in-flight set (the per-session `synced`
        // dedup inside the loop is what the test exercises).
        std::sync::Arc::new(tokio::sync::Mutex::new(())),
        None,
        wid_changed,
        crate::progress::ProgressSink::default(),
        // Tests assert op convergence over loopback responders that mount only
        // SYNC_ALPN; don't fire the extra asset-ALPN pull (asset transfer has its
        // own dedicated regression harness below).
        false,
    )
    .await
}

/// The **real** gossip-topic derivation, exposed so an integration test can
/// assert that two devices at DIFFERENT local paths but with the SAME
/// [`WorkspaceId`] land on the SAME topic (and a different id → different topic).
/// Returns the topic as bytes so the test crate doesn't need the `iroh-gossip`
/// `TopicId` type in scope.
pub fn topic_id_bytes(workspace_id: &WorkspaceId) -> [u8; 32] {
    *crate::engine::workspace_topic_id(workspace_id).as_bytes()
}

/// Build the **real** membership broadcast payload from a `peers.json` file,
/// then parse it back through the **real** receive-side decoder — exactly the
/// round-trip a device performs over gossip. Returns the decoded peer list a
/// receiver would merge (empty when the source has no peers).
///
/// Exercises the production `build_membership_payload` + `parse_membership`
/// without standing up iroh-gossip's loopback swarm (which needs a relay to form
/// reliably). The transitive-merge + re-dial behaviour the test then asserts is
/// the same code the live receive task runs.
pub fn membership_roundtrip(peers_path: &Path) -> Vec<PeerEntry> {
    let Some(payload) = crate::engine_membership::build_membership_payload(peers_path)
        .expect("build membership payload")
    else {
        return Vec::new();
    };
    let content = std::str::from_utf8(&payload).expect("membership payload is utf8");
    crate::engine_membership::parse_membership(content)
        .expect("payload is a membership message")
        .expect("membership payload decodes")
}

/// Merge a gossiped peer list into `peers_path` via the **real**
/// `merge_membership` (drops self + unreachable, only adds unknown node_ids,
/// persists). Returns the number of peers newly added.
pub fn membership_merge(peers_path: &Path, self_node_id: &str, incoming: Vec<PeerEntry>) -> usize {
    crate::engine_membership::merge_membership(peers_path, self_node_id, incoming)
        .expect("merge membership")
}

/// A connection pool over `endpoint`, so an out-of-crate test can prove the
/// pooling contract (a confirmed sync leaves the connection open; the next
/// sync reuses it) instead of inferring it from timings.
pub fn connection_pool(endpoint: iroh::Endpoint) -> crate::peer_conn::PeerConnections {
    crate::peer_conn::PeerConnections::new(endpoint)
}

/// The pooled connection for `peer`, if the pool holds a live one.
///
/// Returns `None` both when nothing is cached and when the cached entry is no
/// longer usable — the same answer `get_or_connect` acts on, which is what a
/// test asserting "the dead one was dropped" needs to see.
pub fn pooled_connection(
    conns: &crate::peer_conn::PeerConnections,
    peer: iroh::EndpointId,
) -> Option<iroh::endpoint::Connection> {
    conns.live_connection(peer)
}

/// [`run_delta_sync`] over a caller-supplied pool, so consecutive calls can be
/// observed sharing one connection.
pub async fn run_delta_sync_pooled(
    conns: &crate::peer_conn::PeerConnections,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
) -> Result<()> {
    drive_delta_sync(
        conns,
        peer,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        crate::progress::ProgressSink::default(),
    )
    .await
}
