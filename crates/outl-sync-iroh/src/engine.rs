//! IrohSyncTransport — implements SyncTransport using iroh QUIC + iroh-gossip.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use iroh::protocol::Router;
use iroh_gossip::{Gossip, TopicId};
use outl_actions::{PeerHealthSnapshot, SyncTransport};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::WorkspaceId;
use tracing::{debug, info, warn};

use crate::health::PeerHealthMap;

/// A local-op announcement queued from the sync side for the gossip task.
///
/// `(workspace_id, hlc)` — the gossip task formats it as `"workspace_id\nactor\nhlc"`
/// to match the receive-side parser in [`crate::engine_gossip`].
pub(crate) type Announce = (String, Hlc);

use crate::engine_catchup::{catch_up_loop, drain_sync_now};
use crate::engine_pairing::{
    drain_pair_completions, pair_host_on_hub, pair_join_on_hub, PairingHub, PairingProtocolHandler,
};
// The delta-sync wire protocol lives in `engine_sync`; re-exported here so
// `crate::engine::{delta_sync, SyncProtocolHandler}` keeps resolving for the
// catch-up loop, pairing drain, the router below, and `test_support`.
use crate::engine_assets::AssetProtocolHandler;
use crate::engine_snapshot::SnapshotProtocolHandler;
pub(crate) use crate::engine_sync::{delta_sync, SyncProtocolHandler};
use crate::identity::IrohIdentity;
use crate::peers::{PeerEntry, PeersStore};
use crate::protocol::{ASSET_ALPN, PAIRING_ALPN, SNAPSHOT_ALPN, SYNC_ALPN};

/// iroh-based P2P transport.
///
/// Spawns a tokio runtime in a dedicated background thread.
/// All iroh I/O runs inside that runtime; the `SyncTransport` API is sync.
#[derive(Clone)]
pub struct IrohSyncTransport {
    identity: Arc<IrohIdentity>,
    peers: Arc<Mutex<PeersStore>>,
    /// Relay URL for the sync endpoint, from `[sync] relay_url` in the user
    /// config. `None` (or empty, normalized to `None` by `SyncConfig::relay_url`)
    /// uses outl's default relay (`use1-1.relay.avelino.outl.iroh.link`); `Some(url)` swaps in a
    /// different relay via [`crate::bind::n0_builder_ipv4_only`]. Only the
    /// long-lived sync endpoint threads it; pairing / status / test endpoints
    /// pass `None` and resolve the same `use1-1.relay.avelino.outl.iroh.link` default.
    relay_url: Option<String>,
    /// Sender used to trigger graceful shutdown.
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// Sender that pushes local-op announcements into the gossip task.
    ///
    /// Populated by `start()`; `announce_local_ops` sends through it. `None`
    /// before the transport starts (or after the runtime tears down).
    announce_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Announce>>>>,
    /// Sender that triggers an immediate forced sync pass against all peers.
    ///
    /// Populated by `start()`; `sync_now()` sends a unit through it, drained by
    /// the `drain_sync_now` task in `run_iroh`. `None` before the transport
    /// starts (or after the runtime tears down), so `sync_now()` is a no-op when
    /// nothing is running — same guard shape as `announce_tx`.
    sync_now_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<()>>>>,
    /// Per-peer reachability, written by the transport's own dials (boot
    /// connect, catch-up loop, gossip-triggered sync, inbound serve) and read
    /// by `peer_health()` for the GUI status indicator.
    ///
    /// This is the whole reason the status path no longer binds a transient
    /// probe endpoint: a second endpoint sharing the device identity hijacks
    /// the relay route from this long-lived sync endpoint. See `crate::health`.
    health: PeerHealthMap,
    /// Pairing coordinator, published by `run_iroh` once the live endpoint is
    /// bound. `pair_host` / `pair_join` reach the live endpoint + peer store
    /// through it, so GUI pairing reuses the one sync endpoint instead of
    /// binding a second one with the same identity. See `crate::engine_pairing`.
    /// `None` until the transport has started (and again after shutdown).
    pairing_hub: Arc<Mutex<Option<Arc<PairingHub>>>>,
    /// Count of **completed forced sync passes** (each `sync_now` request that
    /// finished its full dial cycle over every peer in `peers.json`).
    ///
    /// Incremented by the `drain_sync_now` task when `force_sync_all` returns —
    /// i.e. every peer dial in that pass either succeeded or failed. Read via
    /// [`Self::completed_sync_passes`] so a caller that fired `sync_now()` can
    /// observe completion (snapshot before, poll until it advances) instead of
    /// sleeping a fixed worst-case window. This is what lets the iOS background
    /// FFI return early and hand the unused window back to the OS.
    sync_passes: Arc<AtomicU64>,
    /// Sink for [`outl_actions::SyncProgress`] updates, registered by the GUI
    /// bridge via `set_progress_sink` **before** `start()`. Read once in
    /// `start()` to build the [`crate::progress::ProgressSink`] threaded through
    /// the initiator-side sync paths. `None` (no GUI, or the CLI/tests) makes
    /// every progress emit a no-op — it is purely cosmetic, never load-bearing.
    progress_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<outl_actions::SyncProgress>>>>,
    /// This device's endpoint claim, attached by [`crate::build_transport`].
    ///
    /// **`start()` takes it out of here and moves it into the `outl-iroh-sync`
    /// thread**, so the claim ends exactly when `run_iroh` returns — and the
    /// reason is that neither neighbour of that instant is safe.
    ///
    /// *Released too early* — the transport holding it and being dropped while
    /// the thread is still inside `router.shutdown()` + `endpoint.close()`.
    /// [`SyncTransport::shutdown`] only sends the oneshot and returns, so a
    /// desktop workspace switch drops the transport immediately afterwards:
    /// another process could take the lease and bind while the outgoing
    /// endpoint is still registered on the relay. Two endpoints on one node id
    /// is the collision this lease exists to prevent.
    ///
    /// *Released too late* — the transport holding it after the thread is gone.
    /// `bind()` is the only `?` in `run_iroh`, so a failed bind kills the thread
    /// while the client keeps the transport in its slot for the rest of the
    /// process. Nothing on the device could ever bind again, and the MCP server
    /// never re-asks (it early-returns on a populated slot): issue #220 back,
    /// this time with a padlock on it.
    ///
    /// Owning the lease from the thread makes both impossible: the endpoint and
    /// the claim on it live and die in the same scope.
    ///
    /// `None` for a transport built directly through [`IrohSyncTransport::new`]
    /// (tests, `pair`), which is the caller promising no other endpoint is up,
    /// and `None` again once `start()` has taken it. A transport that is built
    /// but **never** started keeps holding it (the `outl sync` path that exits
    /// early), which is what we want: no other process may bind while this one
    /// still might.
    endpoint_lease: Arc<Mutex<Option<crate::lease::EndpointLease>>>,
}

/// Process-wide guard serializing every op-log append performed by the iroh
/// transport.
///
/// `ingest_received_ops` opens `ops-<actor>.jsonl` in append mode and writes a
/// batch. Three concurrent paths reach it for the *same* file — boot connect,
/// the 8s catch-up loop, gossip-triggered sync (all via `delta_sync`), plus the
/// inbound `serve` side. Without serialization two `write_all`s interleave at
/// the syscall layer and glue two ops together with no separating newline
/// (`…}}}{"ts":…`), corrupting the log. A single global async mutex held across
/// the open+write+flush of each batch closes that race. Batches are small and
/// infrequent, so a global lock costs nothing measurable and correctness wins.
pub(crate) type AppendLock = Arc<tokio::sync::Mutex<()>>;

/// Process-wide set of peers with a `delta_sync` currently in flight.
///
/// Defense in depth on top of [`AppendLock`]: boot + catch-up + gossip can all
/// launch a `delta_sync` for the same peer at once. Each redundant run dials,
/// re-exchanges the full delta, and queues another writer behind the append
/// lock. Skipping a dial when one is already running for that peer cuts the
/// redundant relay traffic and the pile-up of writers.
pub(crate) type InFlightPeers = Arc<std::sync::Mutex<HashSet<iroh::EndpointId>>>;

/// Shared, mutable handle to this workspace's stable [`WorkspaceId`].
///
/// Read at call time by every `delta_sync` / serve so the value reflects the
/// **current** workspace identity, and written by pairing adoption (the joiner
/// overwrites its id with the host's — see `engine_pairing`). Because the
/// initiator dials and the responder validates against this same live value, an
/// adopted id takes effect for the immediate post-pair sync and every later sync
/// without a transport restart. (The gossip *topic* is subscribed once at boot
/// from the boot-time id; an adopted id reaches real-time gossip on the next
/// start, but direct delta-sync — boot connect, 8s catch-up, immediate post-pair
/// dial — carries it live, so content still converges immediately. See the
/// crate `CLAUDE.md`.)
pub(crate) type SharedWorkspaceId = Arc<std::sync::RwLock<WorkspaceId>>;

/// RAII guard that removes a peer from the in-flight set on drop, so an early
/// return or an error inside `delta_sync` never leaves a peer stuck "in flight".
pub(crate) struct InFlightGuard {
    peers: InFlightPeers,
    nid: iroh::EndpointId,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.peers.lock() {
            set.remove(&self.nid);
        }
    }
}

/// Try to mark `nid` in flight. Returns `Some(guard)` if it was free (caller
/// proceeds and the guard clears it on drop), `None` if a sync is already
/// running for that peer (caller skips).
pub(crate) fn try_acquire_in_flight(
    peers: &InFlightPeers,
    nid: iroh::EndpointId,
) -> Option<InFlightGuard> {
    let mut set = peers.lock().ok()?;
    if set.insert(nid) {
        Some(InFlightGuard {
            peers: peers.clone(),
            nid,
        })
    } else {
        None
    }
}

impl std::fmt::Debug for IrohSyncTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrohSyncTransport")
            .field("node_id", &self.identity.node_id().fmt_short().to_string())
            .finish()
    }
}

impl IrohSyncTransport {
    /// Create a new transport. Call `SyncTransport::start` to activate it.
    ///
    /// `relay_url` comes from `[sync] relay_url` in the user config
    /// (`outl_config::SyncConfig::relay_url`). `None` uses outl's default relay
    /// (`use1-1.relay.avelino.outl.iroh.link`); `Some(url)` points the long-lived sync endpoint at a
    /// different relay.
    pub fn new(identity: IrohIdentity, peers: PeersStore, relay_url: Option<String>) -> Self {
        Self {
            identity: Arc::new(identity),
            peers: Arc::new(Mutex::new(peers)),
            relay_url,
            shutdown_tx: Arc::new(Mutex::new(None)),
            announce_tx: Arc::new(Mutex::new(None)),
            sync_now_tx: Arc::new(Mutex::new(None)),
            health: PeerHealthMap::default(),
            pairing_hub: Arc::new(Mutex::new(None)),
            sync_passes: Arc::new(AtomicU64::new(0)),
            progress_tx: Arc::new(Mutex::new(None)),
            endpoint_lease: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach the device endpoint lease this transport was authorised by.
    ///
    /// Called only from [`crate::build_transport`], which is the one place that
    /// acquires a lease. The transport parks it until `start()` hands it to the
    /// endpoint thread, so a client never has to keep a separate guard alive —
    /// and never gets the chance to drop one while its endpoint keeps running.
    /// See the `endpoint_lease` field for why the thread, and not the
    /// transport, is where the claim ends.
    pub(crate) fn with_endpoint_lease(self, lease: crate::lease::EndpointLease) -> Self {
        *self
            .endpoint_lease
            .lock()
            .expect("endpoint lease mutex poisoned") = Some(lease);
        self
    }

    /// How many forced sync passes have **completed** since the transport was
    /// created.
    ///
    /// A pass is one drained `sync_now` request: `force_sync_all` dialed every
    /// peer currently in `peers.json` and every dial either succeeded or
    /// failed (the dial *cycle* finished — it does not promise each peer was
    /// reachable). Callers that need "my forced sync landed" snapshot this
    /// before calling [`SyncTransport::sync_now`] and poll until the value
    /// advances; any pass completing after the snapshot implies a full dial
    /// cycle ran after the request. Monotonic; `0` before the first pass.
    pub fn completed_sync_passes(&self) -> u64 {
        self.sync_passes.load(Ordering::Acquire)
    }

    /// Host one pairing session over the **live sync endpoint** and return the
    /// stored [`PeerEntry`] once a device completes the handshake.
    ///
    /// `on_ticket` fires synchronously the moment the ticket is known, so the
    /// GUI can render the QR while the user walks it to the second device; the
    /// future then resolves on a successful pair (or a timeout). The handshake
    /// runs on the transport's own tokio runtime via the live endpoint's
    /// [`PAIRING_ALPN`] router handler — **no second endpoint is bound**, so the
    /// relay route the sync endpoint owns is never hijacked.
    ///
    /// Returns an error if the transport hasn't started yet (no live endpoint to
    /// pair through). GUI clients call this instead of [`crate::host_pairing`].
    pub async fn pair_host<F>(&self, alias: Option<String>, on_ticket: F) -> Result<PeerEntry>
    where
        F: FnOnce(&str) + Send + 'static,
    {
        let hub = self.require_hub()?;
        pair_host_on_hub(hub, alias, on_ticket).await
    }

    /// Join a pairing session from a host's `ticket` over the **live sync
    /// endpoint**, persisting the host as a peer and kicking an immediate sync.
    ///
    /// Like [`Self::pair_host`], this dials out over the one long-lived endpoint
    /// (no second bind). Returns an error if the transport hasn't started yet.
    /// GUI clients call this instead of [`crate::join_pairing`].
    pub async fn pair_join(&self, ticket: String, alias: Option<String>) -> Result<PeerEntry> {
        let hub = self.require_hub()?;
        pair_join_on_hub(hub, ticket, alias).await
    }

    /// Fetch the live pairing hub, or error if the transport hasn't started.
    fn require_hub(&self) -> Result<Arc<PairingHub>> {
        self.pairing_hub
            .lock()
            .expect("pairing hub mutex poisoned")
            .clone()
            .context("iroh transport not started yet; cannot pair (no live endpoint)")
    }

    /// List known peers.
    pub fn peers(&self) -> Vec<PeerEntry> {
        self.peers
            .lock()
            .expect("peers mutex poisoned")
            .list()
            .to_vec()
    }

    /// Remove a peer by node_id prefix. Returns true if removed.
    pub fn remove_peer(&self, prefix: &str) -> Result<bool> {
        self.peers
            .lock()
            .expect("peers mutex poisoned")
            .remove(prefix)
    }
}

impl SyncTransport for IrohSyncTransport {
    fn start(
        &self,
        workspace_root: PathBuf,
        actor: ActorId,
        peer_ready_tx: std::sync::mpsc::Sender<()>,
    ) {
        let identity = self.identity.clone();
        let peers = self.peers.clone();
        let health = self.health.clone();
        let pairing_hub = self.pairing_hub.clone();
        let relay_url = self.relay_url.clone();
        let sync_passes = self.sync_passes.clone();

        // The one field that is MOVED, not cloned: the device endpoint lease
        // belongs to the endpoint, and the endpoint lives on the thread below.
        // Leaving it on the transport strands it when `run_iroh` dies on a
        // failed `bind()`, and releases it too early when the client drops the
        // transport right after `shutdown()`. See the field doc.
        let endpoint_lease = self
            .endpoint_lease
            .lock()
            .expect("endpoint lease mutex poisoned")
            .take();

        // Snapshot the progress sink registered by the GUI bridge (if any).
        // Purely cosmetic: a missing sink makes every progress emit a no-op.
        let progress = match self
            .progress_tx
            .lock()
            .expect("progress mutex poisoned")
            .clone()
        {
            Some(tx) => crate::progress::ProgressSink::new(tx),
            None => crate::progress::ProgressSink::default(),
        };

        // Resolve the STABLE, SHARED workspace identity once, before binding.
        // Generated + persisted at `<root>/.outl/workspace-id` on first open
        // (migration path for existing workspaces); the same bytes on every
        // paired device. This is what the gossip topic + sync request key on,
        // NOT the local path (which differs per device). A read/create failure
        // falls back to an ephemeral id so the transport still boots; it just
        // won't agree with peers until the file is readable.
        let workspace_id = match WorkspaceId::read_or_create(&workspace_root) {
            Ok(id) => id,
            Err(e) => {
                warn!("workspace id read/create failed ({e}); using ephemeral id");
                WorkspaceId::new()
            }
        };

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().expect("shutdown mutex poisoned") = Some(shutdown_tx);

        // Bridge the sync side (`announce_local_ops`) to the tokio gossip task.
        let (announce_tx, announce_rx) = tokio::sync::mpsc::unbounded_channel::<Announce>();
        *self.announce_tx.lock().expect("announce mutex poisoned") = Some(announce_tx);

        // Bridge the sync side (`sync_now`) to the forced-sync drain task.
        let (sync_now_tx, sync_now_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        *self.sync_now_tx.lock().expect("sync_now mutex poisoned") = Some(sync_now_tx);

        std::thread::Builder::new()
            .name("outl-iroh-sync".into())
            .spawn(move || {
                // Declared FIRST so it is dropped LAST: the runtime below drops
                // before it, so the endpoint is closed and its tasks are gone by
                // the time the device's claim is released.
                let _endpoint_lease = endpoint_lease;

                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime for iroh sync");

                let runtime_handle = rt.handle().clone();
                rt.block_on(async move {
                    if let Err(e) = run_iroh(
                        identity,
                        peers,
                        health,
                        pairing_hub,
                        relay_url,
                        sync_passes,
                        runtime_handle,
                        workspace_root,
                        workspace_id,
                        actor,
                        peer_ready_tx,
                        progress,
                        announce_rx,
                        sync_now_rx,
                        &mut shutdown_rx,
                    )
                    .await
                    {
                        warn!("iroh sync exited with error: {e:#}");
                    }
                });
            })
            .expect("spawn outl-iroh-sync thread");
    }

    fn set_progress_sink(&self, tx: std::sync::mpsc::Sender<outl_actions::SyncProgress>) {
        // Stash it; `start()` reads it once to build the threaded `ProgressSink`.
        // Registering after `start()` has no effect (the sink was already
        // snapshotted) — the GUI bridge always calls this first, by contract.
        *self.progress_tx.lock().expect("progress mutex poisoned") = Some(tx);
    }

    fn announce_local_ops(&self, workspace_id: &str, hlc: outl_core::hlc::Hlc) {
        // Two wake-up paths, fired together, because either one alone is too
        // weak for reliable real-time propagation:
        //
        // 1. Gossip announce — light, but only reaches peers already joined to
        //    the gossip swarm. Across different networks the swarm often hasn't
        //    formed (the flaky iroh 1.0 multipath connect), so the announce
        //    never crosses and the edit only lands on the peer's next catch-up
        //    tick — what felt like "sync is slow, I had to hit refresh".
        // 2. A forced sync pass — dials every known peer directly and runs the
        //    bidirectional delta-sync, PUSHING the new ops without depending on
        //    the gossip swarm. This is exactly what the manual refresh button
        //    does, now fired automatically on every commit so desktop→mobile
        //    propagates on its own. The in-flight guard + cheap no-op delta-sync
        //    (matching vector clocks) keep a burst of edits from piling up dials.
        if let Some(tx) = self
            .announce_tx
            .lock()
            .expect("announce mutex poisoned")
            .as_ref()
        {
            let _ = tx.send((workspace_id.to_string(), hlc));
        }
        if let Some(tx) = self
            .sync_now_tx
            .lock()
            .expect("sync_now mutex poisoned")
            .as_ref()
        {
            let _ = tx.send(());
        }
    }

    fn shutdown(&self) {
        // Signal only — the teardown (`router.shutdown()` + `endpoint.close()`)
        // happens on the sync thread after this returns, and the endpoint lease
        // is released there, at the end of it. Callers routinely drop the
        // transport the moment this returns (desktop workspace switch); that
        // must NOT hand the device's endpoint to another process while this one
        // is still on the relay. See the `endpoint_lease` field.
        if let Some(tx) = self
            .shutdown_tx
            .lock()
            .expect("shutdown mutex poisoned")
            .take()
        {
            let _ = tx.send(());
        }
    }

    fn peer_health(&self) -> Vec<PeerHealthSnapshot> {
        self.health.snapshot()
    }

    fn sync_now(&self) {
        // Hand the request to the forced-sync drain task on the tokio runtime.
        // A send error means the runtime is down (transport stopped or never
        // started) — nothing to sync, so it's ignored. Same no-op-when-down
        // contract as `announce_local_ops`.
        if let Some(tx) = self
            .sync_now_tx
            .lock()
            .expect("sync_now mutex poisoned")
            .as_ref()
        {
            let _ = tx.send(());
        }
    }
}

// ── Core iroh async loop ─────────────────────────────────────────────────────

// Internal orchestration fn: it threads the identity, peer store, health map,
// workspace root, actor, and the three channels into one async loop. Splitting
// the arg list into a struct would add a one-use type for no clarity.
#[allow(clippy::too_many_arguments)]
async fn run_iroh(
    identity: Arc<IrohIdentity>,
    peers: Arc<Mutex<PeersStore>>,
    health: PeerHealthMap,
    pairing_hub_slot: Arc<Mutex<Option<Arc<PairingHub>>>>,
    relay_url: Option<String>,
    sync_passes: Arc<AtomicU64>,
    runtime: tokio::runtime::Handle,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    progress: crate::progress::ProgressSink,
    announce_rx: tokio::sync::mpsc::UnboundedReceiver<Announce>,
    sync_now_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    shutdown_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    // Shared, live workspace identity. `delta_sync`/serve read it at call time,
    // and pairing adoption (via the hub) writes it, so a joiner that adopts the
    // host's id starts syncing as that workspace immediately.
    let workspace_id: SharedWorkspaceId = Arc::new(std::sync::RwLock::new(workspace_id));
    // Build the iroh endpoint with our identity and the n0 discovery preset.
    //
    // Advertise ALL the ALPNs on the ONE endpoint (one endpoint per identity —
    // a separate endpoint would hijack the relay route and kill sync):
    // `SYNC_ALPN` for op-sync, `PAIRING_ALPN` so GUI pairing rides this same
    // endpoint, `SNAPSHOT_ALPN` so a freshly-paired peer can pull this device's
    // materialized snapshot (Phase 2 snapshot sync), and `ASSET_ALPN` so peers
    // can transfer content-addressed binary assets (uploaded files) that never
    // enter the op log (see `crate::engine_assets`).
    //
    // STOPGAP: IPv4-only bind. iroh 1.0.0 multipath stalls on unreachable IPv6
    // direct paths; binding IPv4-only stops this endpoint from advertising a
    // dead global IPv6 addr to peers. Relay + LAN-IPv4 direct stay. Revert to
    // the plain dual-stack builder when iroh > 1.0.0 ships the multipath
    // fallback fix. See `crate::bind`.
    //
    // `relay_url` (from `[sync] relay_url`) selects the relay this long-lived
    // endpoint registers with: `None` uses outl's `use1-1.relay.avelino.outl.iroh.link` default,
    // `Some(url)` swaps in a different relay. Pairing / status / test endpoints
    // pass `None` and resolve the same default.
    let endpoint = crate::bind::n0_builder_ipv4_only(relay_url.as_deref())
        .secret_key(identity.secret_key().clone())
        .alpns(vec![
            SYNC_ALPN.to_vec(),
            PAIRING_ALPN.to_vec(),
            SNAPSHOT_ALPN.to_vec(),
            ASSET_ALPN.to_vec(),
        ])
        .bind()
        .await
        .context("bind iroh endpoint")?;

    info!(node_id = %endpoint.id().fmt_short(), "iroh endpoint bound");

    // Process-wide append guard shared by every writer (boot, catch-up, gossip,
    // and the inbound serve side) so two batches never interleave on the same
    // ops-<actor>.jsonl. This is the load-bearing fix for the `}}}{` glued-op
    // corruption. See `AppendLock`.
    let append_lock: AppendLock = Arc::new(tokio::sync::Mutex::new(()));
    // Defense in depth: skip launching a second delta_sync for a peer that
    // already has one running. See `InFlightPeers`.
    let in_flight: InFlightPeers = Arc::new(std::sync::Mutex::new(HashSet::new()));

    // Build gossip.
    let gossip = Gossip::builder().spawn(endpoint.clone());

    // The on-disk path peers.json lives at, so the catch-up loop below can
    // reload peers added by pairing AFTER this transport booted.
    let peers_path = peers
        .lock()
        .expect("peers mutex poisoned")
        .path()
        .to_path_buf();

    // Publish the pairing hub so the GUI's `pair_host` / `pair_join` can reach
    // the live endpoint. The router's PAIRING_ALPN handler shares the same hub,
    // so an inbound pairing dial completes the host-side handshake here instead
    // of on a second endpoint. `pair_done_rx` drives an immediate sync against
    // each freshly paired peer (see `drain_pair_completions`).
    let (pairing_hub, pair_done_rx, wid_changed_rx) = PairingHub::new(
        endpoint.clone(),
        identity.clone(),
        peers_path.clone(),
        runtime.clone(),
        workspace_root.clone(),
        workspace_id.clone(),
    );
    *pairing_hub_slot.lock().expect("pairing hub mutex poisoned") = Some(pairing_hub.clone());

    // Build router — registers gossip + our sync protocol + pairing.
    let router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(
            SYNC_ALPN,
            SyncProtocolHandler {
                workspace_root: workspace_root.clone(),
                workspace_id: workspace_id.clone(),
                actor,
                peer_ready_tx: peer_ready_tx.clone(),
                append_lock: append_lock.clone(),
            },
        )
        .accept(
            PAIRING_ALPN,
            PairingProtocolHandler {
                hub: pairing_hub.clone(),
            },
        )
        .accept(
            SNAPSHOT_ALPN,
            SnapshotProtocolHandler {
                workspace_root: workspace_root.clone(),
                actor,
            },
        )
        .accept(
            ASSET_ALPN,
            AssetProtocolHandler {
                workspace_root: workspace_root.clone(),
            },
        )
        .spawn();

    // Drain pair-completions: dial each freshly paired peer once for an
    // immediate delta_sync (reuses the append lock + in-flight guard + health
    // map) so a new device syncs without waiting for the 8s catch-up tick.
    let pair_sync = tokio::spawn(drain_pair_completions(
        pair_done_rx,
        endpoint.clone(),
        workspace_root.clone(),
        workspace_id.clone(),
        actor,
        peer_ready_tx.clone(),
        health.clone(),
        append_lock.clone(),
        in_flight.clone(),
        progress.clone(),
    ));

    // Connect to known peers and trigger an initial delta sync. Prefer the full
    // EndpointAddr (id + relay) over the bare id so the connect is reliable.
    let known_peers: Vec<_> = peers.lock().expect("peers mutex poisoned").list().to_vec();
    for peer in &known_peers {
        let Ok(addr) = peer.iroh_endpoint_addr() else {
            continue;
        };
        let ep = endpoint.clone();
        let wr = workspace_root.clone();
        let wid = workspace_id.clone();
        let tx = peer_ready_tx.clone();
        let health = health.clone();
        let lock = append_lock.clone();
        let in_flight = in_flight.clone();
        let prog = progress.clone();
        let nid = addr.id;
        let has_direct = !addr.is_empty();
        tokio::spawn(async move {
            let Some(_in_flight) = try_acquire_in_flight(&in_flight, nid) else {
                debug!(
                    "boot: sync to {} already in flight, skipping",
                    nid.fmt_short()
                );
                return;
            };
            info!(
                peer = %nid.fmt_short(),
                has_direct_addrs = has_direct,
                "boot: connecting to peer for initial sync"
            );
            let started = Instant::now();
            let wid_snapshot = wid.read().expect("workspace id rwlock poisoned").clone();
            match delta_sync(&ep, addr, &wr, &wid_snapshot, actor, tx, &lock, &prog).await {
                Ok(()) => {
                    info!("boot: initial sync to {} ok", nid.fmt_short());
                    health.record_success(nid, started);
                }
                Err(e) => {
                    warn!("boot: initial sync to {} failed: {e}", nid.fmt_short());
                    health.record_failure(nid);
                }
            }
        });
    }

    // Gossip supervisor: real-time op-announce + mesh-membership over the topic
    // derived from the STABLE, SHARED workspace id (NOT the local path — that
    // differs per device and is what broke cross-device gossip). Two paired
    // devices share one id, so they land on the same topic.
    //
    // Unlike the old fire-and-forget subscribe, this is a supervisor task that
    // RE-SUBSCRIBES when the workspace id changes at runtime: a joiner that pairs
    // after boot adopts the host's id, and the supervisor swaps to `blake3(new
    // id)` so live gossip flows without a restart (item 1 of the resume-sync
    // fix). It also runs even with zero peers at boot, so a device that pairs
    // later still gets a live subscription via the id-change path. Reuses the
    // same `Gossip`/`Endpoint` — one endpoint per identity stays intact. See
    // `crate::engine_gossip`.
    let gossip_ctx = crate::engine_gossip::GossipCtx {
        gossip: gossip.clone(),
        endpoint: endpoint.clone(),
        workspace_root: workspace_root.clone(),
        workspace_id: workspace_id.clone(),
        actor,
        peer_ready_tx: peer_ready_tx.clone(),
        health: health.clone(),
        append_lock: append_lock.clone(),
        in_flight: in_flight.clone(),
        peers_path: peers_path.clone(),
        progress: progress.clone(),
    };
    let gossip_task = tokio::spawn(crate::engine_gossip::run_gossip(
        gossip_ctx,
        announce_rx,
        wid_changed_rx,
    ));

    // Periodic catch-up loop: pick up peers paired AFTER boot and pull their
    // full history. The boot-time connect above only saw peers.json as it was
    // at start(); a device paired later writes to the same file but the running
    // transport never re-reads it, so its op-log history is never pulled (only
    // brand-new ops trickle in via gossip). This loop closes that gap.
    let catchup_ep = endpoint.clone();
    let catchup_wr = workspace_root.clone();
    let catchup_wid = workspace_id.clone();
    let catchup_tx = peer_ready_tx.clone();
    let catchup_health = health.clone();
    let catchup_lock = append_lock.clone();
    let catchup_in_flight = in_flight.clone();
    let catchup_peers_path = peers_path.clone();
    let catchup_progress = progress.clone();
    // A second receiver on the same broadcast channel the gossip supervisor uses:
    // when the joiner adopts the host's id, the catch-up loop clears its
    // per-session `synced` dedup so it re-dials every peer under the new id (item
    // 2 of the resume-sync fix). Without this, the single immediate post-pair
    // sync marks the peer synced and the loop never re-dials it again.
    let catchup_wid_changed = pairing_hub.subscribe_wid_changed();
    let catchup = tokio::spawn(async move {
        catch_up_loop(
            catchup_ep,
            catchup_peers_path,
            catchup_wr,
            catchup_wid,
            actor,
            catchup_tx,
            catchup_health,
            catchup_lock,
            catchup_in_flight,
            catchup_wid_changed,
            catchup_progress,
        )
        .await;
    });

    // Forced-sync drain: service GUI "sync now" / pull-to-refresh requests by
    // running an immediate delta_sync pass over every peer (reuses the append
    // lock + in-flight guard + health map). Without this the user's only way to
    // pull was to wait for the 8s catch-up tick. Each completed pass bumps
    // `sync_passes` so `completed_sync_passes()` observers (the iOS background
    // FFI) can return early instead of sleeping a worst-case window. See
    // `drain_sync_now`.
    let sync_now = tokio::spawn(drain_sync_now(
        sync_now_rx,
        endpoint.clone(),
        peers_path,
        workspace_root.clone(),
        workspace_id.clone(),
        actor,
        peer_ready_tx.clone(),
        health.clone(),
        append_lock.clone(),
        in_flight.clone(),
        sync_passes,
        progress.clone(),
    ));

    // Incoming sync connections are handled by the Router above.
    // Wait for the shutdown signal.
    let _ = shutdown_rx.await;
    catchup.abort();
    sync_now.abort();
    pair_sync.abort();
    gossip_task.abort();
    // Drop the published hub: the endpoint is about to close, so any later
    // `pair_host` / `pair_join` must error ("not started") rather than touch a
    // dead endpoint.
    *pairing_hub_slot.lock().expect("pairing hub mutex poisoned") = None;
    router.shutdown().await.ok();
    endpoint.close().await;
    Ok(())
}

/// Compute a deterministic gossip topic id from the STABLE, SHARED workspace id.
///
/// Keyed on [`WorkspaceId`], NOT the local path: two paired devices live at
/// different paths but share one id, so they must land on the same topic. This
/// is the load-bearing fix for the cross-device gossip bug — the old
/// `blake3(workspace_root)` produced a different topic per device, so gossip
/// never connected between real devices. `pub(crate)` so the integration test
/// can assert "same id, different paths → same topic".
pub(crate) fn workspace_topic_id(workspace_id: &WorkspaceId) -> TopicId {
    let hash = blake3::hash(workspace_id.as_str().as_bytes());
    TopicId::from_bytes(*hash.as_bytes())
}
