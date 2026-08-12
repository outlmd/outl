//! Periodic catch-up loop: pull history from peers paired (or recovered) after
//! the transport booted.
//!
//! Extracted from `engine.rs` so that module stays focused on the boot
//! orchestration + the delta-sync wire code. The loop reloads `peers.json` each
//! tick and re-dials new / failed peers; see [`run_catch_up`] for the dedup +
//! retry contract.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use outl_core::id::ActorId;
use tracing::{debug, info, warn};

use crate::engine::{
    delta_sync, try_acquire_in_flight, AppendLock, InFlightPeers, SharedWorkspaceId,
};
use crate::health::PeerHealthMap;
use crate::peers::PeersStore;

/// How often the catch-up loop reloads `peers.json` and re-syncs new/failed
/// peers. Short enough that a freshly paired device syncs within a few seconds,
/// long enough that healthy already-synced peers aren't re-dialed needlessly
/// (those rely on gossip for live updates instead).
const CATCH_UP_INTERVAL: Duration = Duration::from_secs(8);

/// How long a peer that synced cleanly stays "fresh" before the catch-up loop
/// re-dials it as a **maintenance re-sync**.
///
/// This is the safety net that makes convergence independent of the real-time
/// gossip path: even if a peer's op-announce never crosses (flaky cross-network
/// iroh, or a client that never called `announce_local_ops`), the loop re-pulls
/// from every known peer at least this often. `delta_sync` is a cheap no-op when
/// the vector clocks already match, and the shared in-flight guard collapses a
/// slow re-dial into the previous one, so a short interval does not thunder.
///
/// Without this, a peer synced once was marked done for the whole session and
/// never re-dialed — so a later edit on the other device never pulled unless
/// gossip happened to deliver the announce (the "paired, first sync worked, then
/// nothing propagates" bug).
const MAINTENANCE_RESYNC: Duration = Duration::from_secs(10);

/// Warm-up ramp: for the first [`WARMUP_TICKS`] ticks after the loop starts,
/// re-dial peers every [`WARMUP_PERIOD`] instead of the steady `period`.
///
/// iOS accepts inbound connections poorly, so the side that actually opens the
/// path is the **mobile dialing outbound** (its dial reveals the current LAN IP
/// / punches the NAT — the "refreshed direct addr for <mobile>" in the logs).
/// At the steady 8s cadence, a failed first dial makes the mobile wait a full
/// interval before retrying, so first-sync after boot took minutes. A fast
/// initial cadence retries within seconds during the critical boot window, then
/// relaxes so there's no steady-state battery/CPU cost. The in-flight guard
/// collapses a re-dial of a peer already being reached, so the fast cadence
/// can't pile up.
const WARMUP_PERIOD: Duration = Duration::from_secs(2);
const WARMUP_TICKS: u32 = 12;

/// Periodic catch-up against peers added (or recovered) after boot.
///
/// Each tick reloads `peers.json` from disk — this is the whole point: a device
/// paired *after* `start()` writes its [`crate::PeerEntry`] there, and the
/// boot-time connect loop never saw it. For every known peer we run
/// [`delta_sync`], which pulls the peer's entire op-log history on a first
/// contact (empty vector clock for that actor) and is a cheap no-op once the
/// clocks match.
///
/// To avoid re-dialing every healthy peer forever, we keep a `HashSet` of
/// node-ids already synced **this session**. A peer is (re)synced only when it
/// is new this session OR its last attempt failed; a peer that synced cleanly is
/// left to gossip for live updates. The first `interval` tick fires immediately
/// (tokio's default), so a peer paired moments after boot syncs within one tick.
///
/// Returns when the task is aborted at shutdown.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn catch_up_loop(
    conns: crate::peer_conn::PeerConnections,
    peers_path: PathBuf,
    workspace_root: PathBuf,
    workspace_id: SharedWorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    health: PeerHealthMap,
    append_lock: AppendLock,
    in_flight: InFlightPeers,
    wid_changed: tokio::sync::broadcast::Receiver<outl_core::WorkspaceId>,
    progress: crate::progress::ProgressSink,
) {
    // Default resolver: reload peers.json each tick and build a full
    // EndpointAddr (id + relay) for every known peer.
    let resolver = move || match PeersStore::load_or_default(&peers_path) {
        Ok(store) => {
            // Enumerate local interfaces once per tick, not once per peer:
            // `iroh_endpoint_addr` would call `getifaddrs` for every entry.
            let ifaces = crate::peers::local_v4_ifaces();
            store
                .list()
                .iter()
                .filter_map(|p| p.iroh_endpoint_addr_with_ifaces(&ifaces).ok())
                .collect::<Vec<_>>()
        }
        Err(e) => {
            debug!("catch-up: reload peers.json failed: {e}");
            Vec::new()
        }
    };
    run_catch_up(
        conns,
        CATCH_UP_INTERVAL,
        MAINTENANCE_RESYNC,
        resolver,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        Some(health),
        append_lock,
        Some(in_flight),
        Some(wid_changed),
        progress,
        // Production: also converge binary assets each successful sync, so an
        // uploaded PDF / image reaches every device continuously (not just at
        // pairing). Best-effort — a peer without the asset ALPN is a fast skip.
        true,
    )
    .await
}

/// Run a single forced delta-sync pass over the current peer set.
///
/// This backs `IrohSyncTransport::sync_now` (GUI pull-to-refresh / "sync now").
/// Unlike [`catch_up_loop`], it does **not** keep a per-session `synced`
/// dedup — the whole point of a manual sync is to re-dial *every* peer right
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

    for addr in addrs {
        let nid = addr.id;
        // Coordinate with boot / catch-up / gossip dials: skip if one is
        // already running for this peer (its result lands anyway).
        let Some(_in_flight) = try_acquire_in_flight(&in_flight, nid) else {
            debug!(
                "sync-now: sync to {} already in flight, skipping",
                nid.fmt_short()
            );
            continue;
        };
        let started = Instant::now();
        let wid_snapshot = workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            .clone();
        match delta_sync(
            &conns,
            addr,
            &workspace_root,
            &wid_snapshot,
            actor,
            peer_ready_tx.clone(),
            &append_lock,
            &progress,
        )
        .await
        {
            Ok(()) => {
                info!("sync-now: sync to {} ok", nid.fmt_short());
                health.record_success(nid, started);
            }
            Err(e) => {
                warn!("sync-now: sync to {} failed: {e}", nid.fmt_short());
                health.record_failure(nid);
            }
        }
    }
}

/// Drain the `sync_now` trigger channel, running a forced [`force_sync_all`]
/// pass on every signal.
///
/// Mirrors the `announce_rx` drain task in [`crate::engine::run_iroh`]: the sync
/// side (`IrohSyncTransport::sync_now`) sends a unit on an unbounded channel,
/// and this task — spawned once on the transport's runtime — services each
/// request. Coalescing is fine: a burst of taps still ends in one converged
/// state because each pass resolves the latest peers + vector clocks.
///
/// After each pass returns, `passes_completed` is bumped (Release) so
/// `IrohSyncTransport::completed_sync_passes` observers can detect "a full
/// dial cycle finished after my request" and stop waiting early — the iOS
/// background FFI polls exactly this instead of sleeping a fixed window.
///
/// Returns when the sender drops (transport shutdown).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn drain_sync_now(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    conns: crate::peer_conn::PeerConnections,
    peers_path: PathBuf,
    workspace_root: PathBuf,
    workspace_id: SharedWorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    health: PeerHealthMap,
    append_lock: AppendLock,
    in_flight: InFlightPeers,
    passes_completed: Arc<AtomicU64>,
    progress: crate::progress::ProgressSink,
) {
    while rx.recv().await.is_some() {
        // Coalesce every request already queued behind this one into a single
        // pass.
        //
        // "Sync everything now" is idempotent, so N of them queued together
        // want one dial cycle, not N. Without this the channel is an unbounded
        // backlog against a serial consumer: the mobile client fires
        // `syncNow()` on a 3s timer while a pass against a peer that needs the
        // relay takes ~20s, so the queue grows without limit and every request
        // lands further behind the one before it. Observed on device as a
        // forced pass logging `requested` 17µs after the previous one reported
        // `ok`, and as `pass #107` — a backlog nothing was ever going to drain,
        // where a fresh request would not run for half an hour and the sync a
        // user just triggered by editing a block was queued behind a hundred
        // stale copies of itself.
        let mut coalesced = 1u64;
        while rx.try_recv().is_ok() {
            coalesced += 1;
        }
        if coalesced > 1 {
            info!("sync-now: forced sync pass requested (coalescing {coalesced} queued requests)");
        } else {
            info!("sync-now: forced sync pass requested");
        }
        force_sync_all(
            conns.clone(),
            peers_path.clone(),
            workspace_root.clone(),
            workspace_id.clone(),
            actor,
            peer_ready_tx.clone(),
            health.clone(),
            append_lock.clone(),
            in_flight.clone(),
            progress.clone(),
        )
        .await;
        // The dial cycle over every peer finished (each dial succeeded or
        // failed). Publish it for `completed_sync_passes()` pollers — once per
        // REQUEST, not once per pass, so a waiter on sequence N still sees its
        // own request satisfied by the pass that absorbed it.
        passes_completed.fetch_add(coalesced, Ordering::Release);
    }
}

/// The catch-up engine, parameterized over how peers are resolved each tick.
///
/// Production passes a resolver that reloads `peers.json`; tests inject a
/// resolver that returns loopback [`iroh::EndpointAddr`]s (with direct addrs),
/// so the dedup + retry behaviour runs over real QUIC without a relay.
///
/// `resolve_peers` is called once per tick and returns the set of peers to
/// consider. A peer already synced cleanly this session is skipped; a new or
/// previously-failed peer is (re)dialed.
/// `health` records each dial's outcome for the GUI status path; tests pass
/// `None` (they assert sync convergence, not the reachability projection).
/// `append_lock` is the process-wide op-log append guard threaded into
/// `delta_sync`; `in_flight` is the shared per-peer in-flight set (tests pass
/// `None` — they exercise convergence over the per-session `synced` dedup).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_catch_up<F>(
    conns: crate::peer_conn::PeerConnections,
    period: Duration,
    resync_after: Duration,
    mut resolve_peers: F,
    workspace_root: PathBuf,
    workspace_id: SharedWorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    health: Option<PeerHealthMap>,
    append_lock: AppendLock,
    in_flight: Option<InFlightPeers>,
    mut wid_changed: Option<tokio::sync::broadcast::Receiver<outl_core::WorkspaceId>>,
    progress: crate::progress::ProgressSink,
    pull_assets: bool,
) where
    F: FnMut() -> Vec<iroh::EndpointAddr>,
{
    // When each peer last completed a delta_sync cleanly this session. A peer is
    // re-dialed once its last success is older than `resync_after` (the
    // maintenance re-sync) — or immediately if it's new / its last attempt
    // failed (absent from the map). This replaces the old "synced once, never
    // again" set that made convergence hostage to the gossip path.
    let mut last_synced: HashMap<iroh::EndpointId, Instant> = HashMap::new();
    // Drives the warm-up ramp: the first `WARMUP_TICKS` iterations wait
    // `WARMUP_PERIOD`, the rest wait `period` (see the constants). Iteration 0
    // fires immediately — the old `tokio::time::interval` fired its first tick
    // at once, and a freshly paired peer must sync without waiting a tick.
    let mut tick_count: u32 = 0;

    loop {
        let elapsed_ticks = tick_count;
        tick_count = tick_count.saturating_add(1);
        let wait = if elapsed_ticks < WARMUP_TICKS {
            period.min(WARMUP_PERIOD)
        } else {
            period
        };

        // Iteration 0 skips the wait (immediate first pass). Otherwise wait the
        // ramped cadence — but if the workspace id is adopted mid-session (the
        // joiner takes the host's id during pairing), clear `synced` and
        // continue: every peer must be re-dialed under the new id, otherwise the
        // single immediate post-pair sync leaves them marked synced forever and
        // later edits never pull. Reuses the same dial/append/in-flight path —
        // only the dedup is reset.
        if elapsed_ticks > 0 {
            match wid_changed.as_mut() {
                Some(rx) => {
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        changed = rx.recv() => {
                            match changed {
                                // Adopted (or lagged — same response): re-dial everyone.
                                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    info!("catch-up: workspace id changed; clearing synced dedup to re-dial");
                                    last_synced.clear();
                                }
                                // Sender dropped (transport shutdown): stop listening,
                                // keep ticking on the sleep alone.
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    wid_changed = None;
                                }
                            }
                            // Loop straight to a fresh tick selection; the cleared
                            // dedup takes effect on the next resolve_peers pass.
                            continue;
                        }
                    }
                }
                None => {
                    tokio::time::sleep(wait).await;
                }
            }
        }

        for addr in resolve_peers() {
            let nid = addr.id;
            // Skip a peer only while its last clean sync is still fresh (younger
            // than `resync_after`). New peers (from pairing), previously-failed
            // ones (absent), and peers due for a maintenance re-sync fall through
            // and get (re)dialed — that re-sync is what propagates new ops when
            // gossip didn't.
            if let Some(last) = last_synced.get(&nid) {
                if last.elapsed() < resync_after {
                    continue;
                }
            }
            // Coordinate with boot/gossip dials: skip if one is already running.
            let _in_flight = match in_flight.as_ref().map(|s| try_acquire_in_flight(s, nid)) {
                Some(None) => continue,
                Some(Some(guard)) => Some(guard),
                None => None,
            };
            let started = Instant::now();
            let wid_snapshot = workspace_id
                .read()
                .expect("workspace id rwlock poisoned")
                .clone();
            match delta_sync(
                &conns,
                addr.clone(),
                &workspace_root,
                &wid_snapshot,
                actor,
                peer_ready_tx.clone(),
                &append_lock,
                &progress,
            )
            .await
            {
                Ok(()) => {
                    info!("catch-up: sync to {} ok", nid.fmt_short());
                    // Stamp the success so this peer is skipped until it goes
                    // stale (`resync_after`), then re-dialed as maintenance.
                    last_synced.insert(nid, Instant::now());
                    if let Some(h) = &health {
                        h.record_success(nid, started);
                    }
                    // Converge binary assets too (best-effort): pull any
                    // content-addressed uploads the peer holds that this device
                    // lacks. A separate `ASSET_ALPN` connection after the op
                    // sync; an error (e.g. a test peer without the ALPN) is a
                    // debug-logged no-op. Tests pass `pull_assets = false`.
                    if pull_assets {
                        match crate::engine_assets::pull_assets_from_peer(
                            conns.endpoint(),
                            addr,
                            &workspace_root,
                            &progress,
                        )
                        .await
                        {
                            Ok(n) if n > 0 => {
                                info!("catch-up: pulled {n} assets from {}", nid.fmt_short())
                            }
                            Ok(_) => {}
                            Err(e) => {
                                debug!("catch-up: asset pull from {} failed: {e}", nid.fmt_short())
                            }
                        }
                    }
                }
                Err(e) => {
                    // Drop any stale timestamp so the next tick retries it now.
                    last_synced.remove(&nid);
                    warn!("catch-up: sync to {} failed: {e}", nid.fmt_short());
                    if let Some(h) = &health {
                        h.record_failure(nid);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Fix-3 guard (iOS background early-exit): every drained `sync_now`
    /// request bumps the completed-pass counter exactly once when its dial
    /// cycle returns — including the zero-peer cycle (empty `peers.json`),
    /// which is the fastest possible pass. `completed_sync_passes()` pollers
    /// (the background FFI) rely on this to stop waiting instead of sleeping
    /// the full window.
    #[tokio::test(flavor = "multi_thread")]
    async fn drain_sync_now_bumps_completed_pass_counter_per_request() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let identity = crate::IrohIdentity::load_or_generate(&tmp.path().join("identity.key"))
            .expect("identity");
        let endpoint = crate::test_support::bind_sync_endpoint(&identity)
            .await
            .expect("bind endpoint");

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let passes = Arc::new(AtomicU64::new(0));
        let wid: SharedWorkspaceId =
            Arc::new(std::sync::RwLock::new(outl_core::WorkspaceId::new()));

        let drain = tokio::spawn(drain_sync_now(
            rx,
            crate::peer_conn::PeerConnections::new(endpoint.clone()),
            // Absent peers.json → load_or_default yields an empty peer set,
            // so each pass is a fast no-dial cycle.
            tmp.path().join("peers.json"),
            tmp.path().to_path_buf(),
            wid,
            ActorId::new(),
            ready_tx,
            PeerHealthMap::default(),
            Arc::new(tokio::sync::Mutex::new(())),
            Arc::new(std::sync::Mutex::new(HashSet::new())),
            passes.clone(),
            crate::progress::ProgressSink::default(),
        ));

        tx.send(()).expect("queue first sync-now");
        tx.send(()).expect("queue second sync-now");

        // Bounded poll — mirrors what the FFI does with the live transport.
        let deadline = Instant::now() + Duration::from_secs(10);
        while passes.load(Ordering::Acquire) < 2 {
            assert!(
                Instant::now() < deadline,
                "sync-now passes never reported completion"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(passes.load(Ordering::Acquire), 2);

        // Dropping the sender ends the drain task cleanly.
        drop(tx);
        drain.await.expect("drain task join");
    }

    /// The property `IrohSyncTransport::sync_now_seq` sells: the drain is FIFO
    /// and bumps the counter **exactly once per request**, so "the counter
    /// reached my sequence number" means *my* request ran — never merely that
    /// somebody's did.
    ///
    /// Do not relax this into "the counter moved". That is what it used to be,
    /// and the iOS background flush read a concurrent foreground pass (the
    /// mobile client fires `sync_now` on a 3s timer) as its own completing,
    /// released its `beginBackgroundTask` assertion ~250ms in, and let iOS
    /// suspend the app with the flush's own request still queued — the flush
    /// ending the sync it existed to finish.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_queued_request_advances_the_counter_by_exactly_one() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let identity = crate::IrohIdentity::load_or_generate(&tmp.path().join("identity.key"))
            .expect("identity");
        let endpoint = crate::test_support::bind_sync_endpoint(&identity)
            .await
            .expect("bind endpoint");

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let passes = Arc::new(AtomicU64::new(0));
        let wid: SharedWorkspaceId =
            Arc::new(std::sync::RwLock::new(outl_core::WorkspaceId::new()));

        let drain = tokio::spawn(drain_sync_now(
            rx,
            crate::peer_conn::PeerConnections::new(endpoint.clone()),
            tmp.path().join("peers.json"),
            tmp.path().to_path_buf(),
            wid,
            ActorId::new(),
            ready_tx,
            PeerHealthMap::default(),
            Arc::new(tokio::sync::Mutex::new(())),
            Arc::new(std::sync::Mutex::new(HashSet::new())),
            passes.clone(),
            crate::progress::ProgressSink::default(),
        ));

        // Queue a burst, the way a 3s foreground timer overlapping a
        // backgrounding flush does.
        const REQUESTS: u64 = 5;
        for _ in 0..REQUESTS {
            tx.send(()).expect("queue sync-now");
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        while passes.load(Ordering::Acquire) < REQUESTS {
            assert!(
                Instant::now() < deadline,
                "queued sync-now requests never all completed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        drop(tx);
        drain.await.expect("drain task join");

        // Exactly `REQUESTS`, never more: an extra bump would make a waiter on
        // sequence N stop early, which is the whole defect.
        assert_eq!(
            passes.load(Ordering::Acquire),
            REQUESTS,
            "the counter must advance once per request, no more"
        );
    }
}
