//! A window onto a transport's reachability map, for tests.
//!
//! `PeerHealthMap` is crate-private, and both sides write to it: a test asking
//! "did the responder notice it was reached" needs the map the responder
//! holds, and one asking "does a broken exchange still count as contact"
//! needs the initiator's. See `crate::health`.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use iroh::protocol::Router;
use outl_core::id::ActorId;
use outl_core::WorkspaceId;

use crate::engine::SyncProtocolHandler;
use crate::protocol::SYNC_ALPN;

use super::authorize_peer;

/// What a transport recorded about who it is in contact with.
///
/// The transport keeps one reachability map and every path writes to it: the
/// dials it makes, the dials it answers, and the outcome of each exchange. It
/// is crate-private, so this is the window onto it.
#[derive(Clone, Default)]
pub struct HealthWindow(crate::health::PeerHealthMap);

impl HealthWindow {
    /// Does the responder currently read `peer` as reachable?
    pub fn reachable(&self, peer: iroh::EndpointId) -> bool {
        let wanted = peer.to_string();
        self.0
            .snapshot()
            .into_iter()
            .any(|h| h.node_id == wanted && h.reachable)
    }

    /// Everything the responder knows, in the shape the GUI status path reads.
    pub fn snapshot(&self) -> Vec<outl_actions::PeerHealthSnapshot> {
        self.0.snapshot()
    }
}

/// [`super::spawn_responder`], plus a handle on the reachability map the
/// responder writes while it serves.
pub fn spawn_responder_with_health(
    endpoint: iroh::Endpoint,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    authorized_peers: &[iroh::EndpointId],
) -> (Router, HealthWindow) {
    for peer in authorized_peers {
        authorize_peer(&workspace_root, *peer);
    }
    let health = crate::health::PeerHealthMap::default();
    let router = Router::builder(endpoint)
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
                health: health.clone(),
            },
        )
        .spawn();
    (router, HealthWindow(health))
}

/// Run the production `delta_sync` initiator, recording into `health` exactly
/// as the transport's own catch-up / gossip / force-sync dials do.
///
/// The verdict is `delta_sync`'s own: a dial that could not connect records a
/// failure, and one that connected records contact — whatever the exchange
/// that follows does.
#[allow(clippy::too_many_arguments)]
pub async fn run_delta_sync_tracking_health(
    health: &HealthWindow,
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &std::path::Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
) -> anyhow::Result<()> {
    super::drive_delta_sync(
        &crate::peer_conn::PeerConnections::with_health(endpoint.clone(), health.0.clone()),
        peer,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        crate::progress::ProgressSink::default(),
    )
    .await
}
