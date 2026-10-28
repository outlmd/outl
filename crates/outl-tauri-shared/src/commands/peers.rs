//! Peer / device management command bodies.
//!
//! These bypass the workspace lock, because the peer list is
//! sync-transport state rather than workspace state: they read and edit
//! `<workspace>/.outl/peers.json` directly via
//! `outl_sync_iroh::PeersStore`.
//!
//! Not the only module that skips it — `asset`, `exec`, `telemetry` and
//! `page_backlinks` each say so for their own reasons. This one is
//! distinctive in *what* it touches instead: a file beside the
//! workspace that the workspace model knows nothing about.
//!
//! **Pairing stays client-side.** `outl_peer_pair_host` returns a
//! `PairedPeerDto` on the desktop but the raw ticket `String` on mobile
//! (established wire contracts); unifying them would change one client's
//! wire format, so only the DTOs and the read/remove/status/sync bodies
//! live here.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::host::AppHost;
use crate::offload;

/// Resolve `root`'s `peers.json` path (`<root>/.outl/peers.json`) and run
/// the one-time global → workspace migration. The peer list is per-GRAPH;
/// only the device identity stays per-install.
///
/// Takes the root rather than the host because all three callers run on
/// the blocking pool, where there is no `AppHost` left to ask — and the
/// migration is a file read plus a possible copy, so it belongs there
/// with the rest of the I/O, not on the IPC thread.
fn peers_path_at(root: &std::path::Path) -> PathBuf {
    outl_sync_iroh::migrate_global_peers_if_absent(root);
    outl_sync_iroh::workspace_peers_path(root)
}

/// DTO for a paired peer.
///
/// `Clone` because the mobile pairing task hands one to
/// `app.emit("peer-paired", …)`, and Tauri's `Emitter::emit` takes the
/// payload by `Clone + Serialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub added_at: String,
}

impl From<outl_sync_iroh::PeerEntry> for PeerDto {
    fn from(p: outl_sync_iroh::PeerEntry) -> Self {
        Self {
            node_id: p.node_id,
            alias: p.alias,
            added_at: p.added_at,
        }
    }
}

/// DTO for a peer's live reachability status.
#[derive(Debug, Clone, Serialize)]
pub struct PeerStatusDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub online: bool,
    pub rtt_ms: Option<u64>,
    /// Ops this device holds that the peer has not confirmed yet.
    /// `None` until the two have exchanged once this session.
    pub pending_ops: Option<u64>,
}

/// List all paired devices.
///
/// Async because it reads `peers.json` (and may run the global →
/// workspace migration) — filesystem work has no business on the IPC
/// thread, however small the file is. See [`crate::offload`].
pub async fn peer_list<S: AppHost>(state: &S) -> Result<Vec<PeerDto>, String> {
    let root = state.storage_root()?;
    offload::run("peer_list", move || {
        let peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path_at(&root))
            .map_err(|e| e.to_string())?;
        Ok(peers.list().iter().cloned().map(PeerDto::from).collect())
    })
    .await
}

/// Remove a peer by node_id prefix; `true` if any matched.
pub async fn peer_remove<S: AppHost>(state: &S, id: String) -> Result<bool, String> {
    let root = state.storage_root()?;
    offload::run("peer_remove", move || {
        let mut peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path_at(&root))
            .map_err(|e| e.to_string())?;
        peers.remove(&id).map_err(|e| e.to_string())
    })
    .await
}

/// Reachability for each paired peer, read from the **running** iroh
/// transport's own dial outcomes — never a fresh probe endpoint.
///
/// Binding a second endpoint with the device identity (the old probe
/// path) hijacks the relay route from the long-lived sync endpoint, so
/// inbound sync connections get refused. Status reads the transport's
/// `peer_health()` snapshot (boot connect / catch-up / gossip / serve
/// fill it) and merges it onto the full peers.json list. A peer the
/// transport hasn't dialed yet — or the case where no iroh transport is
/// wired (file transport) — shows `online = false`.
///
/// **Async, and that is load-bearing, not tidiness.** Both clients poll
/// this every 5s, and the `pending_ops` column goes through
/// `outl_sync_iroh::pending_by_peer`, which opens the op log from
/// scratch (`JsonlStorage::open` → `load_actor_indexes` → per-actor
/// `NodeIndex::load` → `sidecar::load_entries`) and then read
/// `all_ops()` — 342ms per call on a 222k-op workspace, and the poll,
/// the per-peer catch-up and the ingest together made that ~100 calls
/// inside one 10s window. Run synchronously it was on the IPC thread,
/// which on iOS is the UI main thread, so the scene-update watchdog
/// killed the app on every launch before it could paint — nine
/// identical crash reports, all bottoming out in `NodeIndex::insert`
/// on `com.apple.main-thread`.
///
/// Two halves fixed it and both are load-bearing. Moving it to the
/// blocking pool is what stops the `SIGKILL`. Making the read cheap is
/// `outl-sync-iroh`'s side: `oplog_census` now answers from the offset
/// sidecar (keyed by `Hlc`, so its keys *are* the distinct-op set)
/// instead of folding every op, 342ms → 63ms. Without that half the
/// watchdog stops firing and the phone just cooks instead.
pub async fn peer_status<S: AppHost>(state: &S) -> Result<Vec<PeerStatusDto>, String> {
    // Snapshot the transport's per-peer health here, on the calling
    // thread: it is an in-memory read of the running transport's own dial
    // outcomes (no disk, no lock), and taking it now means the blocking
    // job never has to own an `Arc<dyn SyncTransport>`.
    let health: HashMap<String, outl_actions::PeerHealthSnapshot> = state
        .sync_transport()
        .map(|t| {
            t.peer_health()
                .into_iter()
                .map(|h| (h.node_id.clone(), h))
                .collect()
        })
        .unwrap_or_default();
    let root = state.storage_root()?;
    let actor = state.hlc().actor();

    offload::run("peer_status", move || {
        let peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path_at(&root))
            .map_err(|e| e.to_string())?;

        // What each peer still has to receive. Best effort: a read error
        // only costs the column, never the status list.
        let pending: HashMap<String, u64> =
            outl_sync_iroh::pending_by_peer(&root, actor).unwrap_or_default();

        Ok(peers
            .list()
            .iter()
            .map(|p| {
                let h = health.get(&p.node_id);
                PeerStatusDto {
                    node_id: p.node_id.clone(),
                    alias: p.alias.clone(),
                    online: h.map(|h| h.reachable).unwrap_or(false),
                    rtt_ms: h.and_then(|h| h.last_rtt_ms),
                    pending_ops: pending.get(&p.node_id).copied(),
                }
            })
            .collect())
    })
    .await
}

/// Force an immediate P2P sync pass against every paired peer.
///
/// Backs the GUI's "Refresh" / pull-to-refresh affordance: instead of
/// waiting for the iroh transport's catch-up tick, the frontend calls
/// this to dial every peer right now and pull the freshest state, then
/// `reload_workspace` to re-render once the ops have landed.
///
/// No-op (returns `Ok`) when no iroh transport is wired (the file
/// transport has no peer to dial) or its runtime is down.
///
/// **Async, and deliberately *not* wrapped in [`offload::run`].** The
/// body is a mutex acquire plus a send on an unbounded channel
/// (`IrohSyncTransport::sync_now` → `sync_now_seq`); every dial happens
/// on the transport's own runtime, so there is no blocking work here for
/// a `spawn_blocking` hop to move — adding one would cost more
/// wall-clock than it saves. What `async` buys is the hand-off itself:
/// Tauri runs an `async fn` command on its runtime, so the IPC thread
/// (the UI main thread under `wry`) is released the moment this is
/// invoked. Mobile fires this in its very first tick, *before* today's
/// journal, which is the one place that ordering matters.
pub async fn sync_now<S: AppHost>(state: &S) -> Result<(), String> {
    if let Some(t) = state.sync_transport() {
        t.sync_now();
    }
    Ok(())
}
