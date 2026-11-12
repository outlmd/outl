//! The forced sync pass behind `IrohSyncTransport::sync_now`.
//!
//! Split out of `engine_catchup.rs` (file-size guard). The catch-up loop is the
//! periodic safety net; this is the "sync every peer right now" pass that
//! pull-to-refresh, the mobile foreground timer and every mobile background
//! window drive.

use std::path::PathBuf;
use std::time::Instant;

use outl_core::id::ActorId;
use tracing::{debug, info, warn};

use crate::engine::{
    delta_sync, try_acquire_in_flight, AppendLock, InFlightPeers, SharedWorkspaceId,
};
use crate::health::PeerHealthMap;
use crate::peers::PeersStore;

/// Run a single forced delta-sync pass over the current peer set.
///
/// This backs `IrohSyncTransport::sync_now` (GUI pull-to-refresh / "sync now").
/// Unlike the catch-up loop, it does **not** keep a per-session `synced`
/// dedup: the whole point of a manual sync is to re-dial *every* peer right
/// now, including healthy ones the catch-up loop leaves to gossip. `delta_sync`
/// is a cheap no-op when the vector clocks already match, so re-dialing a
/// healthy peer just confirms convergence.
///
/// It still honors the shared safety machinery: the per-peer in-flight guard
/// (skip a peer already being dialed by boot / catch-up / gossip), the
/// process-wide append lock (threaded into `delta_sync`), and health recording
/// for the status dot. Peers are resolved once from `peers.json` at call time.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn force_sync_all(
    conns: crate::peer_conn::PeerConnections,
    peers_path: PathBuf,
    workspace_root: PathBuf,
    workspace_id: SharedWorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    health: PeerHealthMap,
    append_lock: AppendLock,
    in_flight: InFlightPeers,
    progress: crate::progress::ProgressSink,
) {
    let addrs: Vec<iroh::EndpointAddr> = match PeersStore::load_or_default(&peers_path) {
        Ok(store) => {
            let ifaces = crate::peers::local_v4_ifaces();
            store
                .list()
                .iter()
                .filter_map(|p| p.iroh_endpoint_addr_with_ifaces(&ifaces).ok())
                .collect()
        }
        Err(e) => {
            debug!("sync-now: reload peers.json failed: {e}");
            return;
        }
    };
    let label = |nid| crate::peer_label::peer_log_label(&peers_path, nid);
    force_sync_addrs(
        &conns,
        addrs,
        &workspace_root,
        &workspace_id,
        actor,
        &peer_ready_tx,
        Some(&health),
        &append_lock,
        &in_flight,
        &progress,
        &label,
    )
    .await;
}

/// Dial every peer in `addrs` **at the same time** and return once all of
/// them have succeeded or failed.
///
/// Concurrent, not one after another, because the pass is what every mobile
/// background window waits on, and those windows are short: the iOS
/// `BGAppRefreshTask` pass is capped at 12s and the processing and flush
/// passes at 20s. An unreachable peer costs up to 15s (5s direct + 10s relay
/// fallback in `connect_with_fallback`), so with the peers dialed in order a
/// laptop that is asleep at the head of `peers.json` used up the whole refresh
/// window, and the always-on peer behind it was never dialed in the
/// background at all. Run together, a pass costs the slowest single dial
/// instead of the sum, and a reachable peer syncs in the first seconds of the
/// window no matter where it sits in the list.
///
/// Safe to run concurrently for the same reasons boot already does it: the
/// in-flight guard is per peer, and every op-log append still goes through
/// the process-wide `append_lock`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn force_sync_addrs<L, D>(
    conns: &crate::peer_conn::PeerConnections,
    addrs: Vec<iroh::EndpointAddr>,
    workspace_root: &std::path::Path,
    workspace_id: &SharedWorkspaceId,
    actor: ActorId,
    peer_ready_tx: &std::sync::mpsc::Sender<()>,
    health: Option<&PeerHealthMap>,
    append_lock: &AppendLock,
    in_flight: &InFlightPeers,
    progress: &crate::progress::ProgressSink,
    label: &L,
) where
    L: Fn(iroh::EndpointId) -> D + Sync,
    D: std::fmt::Display,
{
    let dials = addrs.into_iter().map(|addr| async move {
        let nid = addr.id;
        // Coordinate with boot / catch-up / gossip dials: skip if one is
        // already running for this peer (its result lands anyway).
        let Some(_in_flight) = try_acquire_in_flight(in_flight, nid) else {
            debug!(
                "sync-now: sync to {} already in flight, skipping",
                label(nid)
            );
            return;
        };
        let started = Instant::now();
        let wid_snapshot = workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            .clone();
        match delta_sync(
            conns,
            addr,
            workspace_root,
            &wid_snapshot,
            actor,
            peer_ready_tx.clone(),
            append_lock,
            progress,
        )
        .await
        {
            Ok(()) => {
                info!("sync-now: sync to {} ok", label(nid));
                if let Some(h) = health {
                    h.record_success(nid, started);
                }
            }
            Err(e) => {
                warn!("sync-now: sync to {} failed: {e}", label(nid));
                if let Some(h) = health {
                    h.record_failure(nid);
                }
            }
        }
    });
    n0_future::join_all(dials).await;
}
