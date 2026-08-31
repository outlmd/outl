//! GUI pairing over the **live sync endpoint** (one endpoint per identity).
//!
//! ## Why this module exists
//!
//! The CLI's [`crate::host_pairing`] / [`crate::join_pairing`] bind a one-shot
//! endpoint with the device identity for the [`PAIRING_ALPN`] handshake. That is
//! safe for the CLI (no transport is running, so there is no relay route to
//! steal), but **fatal for a GUI client** whose sync transport is already up:
//! iroh's relay keeps a single `node_id → endpoint` route, so the second
//! endpoint hijacks it and the live sync endpoint silently stops receiving
//! inbound traffic ("Another endpoint connected with the same endpoint id").
//! Sync dies for as long as the pairing endpoint is alive.
//!
//! The fix is to reuse the **one** long-lived endpoint:
//!
//! - The sync endpoint's [`Router`](iroh::protocol::Router) registers a third
//!   ALPN, [`PAIRING_ALPN`], handled by [`PairingProtocolHandler`]. The handler
//!   runs the host (accept) side of the handshake — but only while the user has
//!   "armed" pairing via [`IrohSyncTransport::pair_host`].
//! - [`IrohSyncTransport::pair_join`] dials the host's ticket over the same live
//!   endpoint.
//!
//! After either side persists the new [`PeerEntry`], it pushes the peer's addr
//! onto the pair-completed channel so [`crate::engine::run_iroh`] fires an
//! immediate [`delta_sync`](crate::engine) — the freshly paired device syncs
//! without waiting for the 8s catch-up tick (and without an app restart).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::sync::oneshot;
use tracing::{info, warn};

use outl_core::WorkspaceId;

use crate::engine::SharedWorkspaceId;
use crate::identity::IrohIdentity;
use crate::pairing::{
    accept_host_handshake, mint_ticket, ready_addr, run_join_handshake, PairingSecret,
};
use crate::peers::{PeerEntry, PeersStore};

/// How long an armed host waits for a joiner before the arm expires and the
/// `pair_host` future resolves with a timeout error. Mirrors the CLI's
/// `HOST_ACCEPT_TIMEOUT`.
const HOST_ACCEPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Shared coordination surface between the GUI `pair_*` methods and the
/// [`PairingProtocolHandler`] mounted on the live sync endpoint's router.
///
/// Populated by [`crate::engine::run_iroh`] once the endpoint is bound; `None`
/// before the transport starts. Cloned into the router handler so an inbound
/// `PAIRING_ALPN` connection can reach the armed host state + the peer store.
pub(crate) struct PairingHub {
    /// The live sync endpoint. Used by `pair_host` to mint a ticket from its
    /// ready addr and by `pair_join` to dial out.
    pub(crate) endpoint: iroh::Endpoint,
    /// Device identity (its `SecretKey` is the endpoint's; used to build our
    /// outgoing pairing payload).
    pub(crate) identity: Arc<IrohIdentity>,
    /// On-disk peers file every paired device is persisted to (the same path
    /// the catch-up loop reloads).
    pub(crate) peers_path: std::path::PathBuf,
    /// Handle to the transport's own tokio runtime (the one that owns the
    /// endpoint's driver). `pair_host` / `pair_join` are called from the GUI's
    /// runtime (Tauri), so they spawn the iroh work back onto *this* handle and
    /// await the result — keeping every endpoint op on the runtime that bound
    /// it.
    runtime: tokio::runtime::Handle,
    /// Local workspace root, so the JOINER can persist an adopted workspace id to
    /// `<root>/.outl/workspace-id`.
    workspace_root: std::path::PathBuf,
    /// Live, shared workspace identity. The host advertises this id in its
    /// pairing payload; the joiner ADOPTS the host's id (writes it to disk + into
    /// this handle) so both sides then derive the same gossip topic and validate
    /// each other's sync requests as one workspace. See the crate `CLAUDE.md`.
    workspace_id: SharedWorkspaceId,
    /// Broadcasts the newly-adopted [`WorkspaceId`] when the joiner adopts the
    /// host's id at runtime. `run_iroh` keeps the receivers and reacts: the
    /// gossip task drops its old-topic subscription and re-subscribes to
    /// `blake3(new id)` (so real-time gossip flows without a restart), and the
    /// catch-up loop clears its per-session `synced` dedup (so it re-dials every
    /// peer under the adopted id). Without this, gossip stays stuck on the
    /// pre-adoption topic and the catch-up loop never re-dials after the single
    /// immediate post-pair sync. See the crate `CLAUDE.md`.
    wid_changed_tx: tokio::sync::broadcast::Sender<WorkspaceId>,
    /// The armed hosting session, and the rule about when it is consumed.
    arm: ArmSlot,
    /// Fires a paired peer's addr so `run_iroh` can dial it immediately.
    pair_done_tx: tokio::sync::mpsc::UnboundedSender<iroh::EndpointAddr>,
}

/// Holds the armed hosting session, and owns the one rule that matters about
/// it: **an inbound connection may read the arm, but only a completed handshake
/// may consume it.**
///
/// That rule used to be a single `take()` at the top of the router handler, and
/// it was wrong in a way nothing could catch. Any inbound dial disarmed the
/// user — a stranger, a stale probe, a joiner whose handshake failed. It was
/// survivable while the handshake could only fail by accident; adding a proof
/// that a stranger is *supposed* to fail turned it into a one-packet way to
/// stop someone pairing at all (issue #159).
///
/// Extracted from `PairingHub` so the rule has a name and a test. The hub needs
/// an endpoint, a runtime handle and a workspace to construct; this needs
/// nothing, which is the difference between the policy being tested and being
/// asserted in a comment.
#[derive(Default)]
struct ArmSlot {
    inner: Mutex<Option<HostArm>>,
    /// Incremented on every [`arm`](Self::arm). Lets a caller say *which*
    /// session it means when disarming.
    ///
    /// Without it, "disarm" is ambiguous the moment two sessions can overlap,
    /// and they do: `pair_host_on_hub` spawns its host-wait onto the
    /// transport's runtime, so a timed-out wait from a **previous** pairing
    /// screen is still alive and still calling `complete`. It would take the
    /// arm belonging to the ticket the user is looking at right now, which
    /// then answers `not-armed`.
    generation: Mutex<u64>,
}

/// A specific armed session. Only the holder of a matching generation may
/// disarm it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ArmGeneration(u64);

impl ArmSlot {
    /// Arm hosting, superseding any previous session (last-armer-wins:
    /// re-opening the pairing screen replaces a stale arm). Returns the
    /// generation identifying *this* session.
    fn arm(&self, arm: HostArm) -> ArmGeneration {
        let mut gen = self.generation.lock().expect("pairing generation poisoned");
        *gen += 1;
        *self.inner.lock().expect("pairing arm mutex poisoned") = Some(arm);
        ArmGeneration(*gen)
    }

    /// What an inbound connection needs to run the handshake, plus the
    /// generation it belongs to. **Does not disarm** — call it as many times as
    /// connections arrive.
    fn snapshot(&self) -> Option<(Option<String>, PairingSecret, ArmGeneration)> {
        let gen = *self.generation.lock().expect("pairing generation poisoned");
        let guard = self.inner.lock().expect("pairing arm mutex poisoned");
        guard
            .as_ref()
            .map(|a| (a.alias.clone(), a.secret.clone(), ArmGeneration(gen)))
    }

    /// Consume the arm **if it is still the session `gen` names**. Only a
    /// handshake that passed every check, or the wait for that exact session
    /// timing out, may call this.
    ///
    /// Returns `None` when the arm was already taken *or* has been superseded,
    /// and those are the same answer to the caller: the session it was holding
    /// is over, and whatever is armed now belongs to someone else.
    fn complete(&self, gen: ArmGeneration) -> Option<HostArm> {
        let current = *self.generation.lock().expect("pairing generation poisoned");
        if current != gen.0 {
            return None;
        }
        self.inner
            .lock()
            .expect("pairing arm mutex poisoned")
            .take()
    }

    /// Whether hosting is currently armed. Test-only: production reads the arm
    /// through [`snapshot`](Self::snapshot), which answers the same question
    /// and returns what the caller needs.
    #[cfg(test)]
    fn is_armed(&self) -> bool {
        self.inner
            .lock()
            .expect("pairing arm mutex poisoned")
            .is_some()
    }
}

/// One armed hosting session: the alias to advertise + the channel that delivers
/// the completed [`PeerEntry`] back to the awaiting `pair_host` future.
struct HostArm {
    alias: Option<String>,
    /// Secret baked into the ticket this arm handed out; the joiner must prove
    /// it holds it (issue #159).
    secret: PairingSecret,
    result_tx: oneshot::Sender<PeerEntry>,
}

impl PairingHub {
    /// Build the hub + the receiver half of the pair-completed channel.
    ///
    /// `run_iroh` keeps the receiver and drains it to fire an immediate
    /// `delta_sync` against each freshly paired peer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        endpoint: iroh::Endpoint,
        identity: Arc<IrohIdentity>,
        peers_path: std::path::PathBuf,
        runtime: tokio::runtime::Handle,
        workspace_root: std::path::PathBuf,
        workspace_id: SharedWorkspaceId,
    ) -> (
        Arc<Self>,
        tokio::sync::mpsc::UnboundedReceiver<iroh::EndpointAddr>,
        tokio::sync::broadcast::Receiver<WorkspaceId>,
    ) {
        let (pair_done_tx, pair_done_rx) = tokio::sync::mpsc::unbounded_channel();
        // A small buffer is plenty: id adoption happens at most once per pairing
        // and consumers (gossip re-subscribe + catch-up) drain it promptly.
        let (wid_changed_tx, wid_changed_rx) = tokio::sync::broadcast::channel(8);
        let hub = Arc::new(Self {
            endpoint,
            identity,
            peers_path,
            runtime,
            workspace_root,
            workspace_id,
            wid_changed_tx,
            arm: ArmSlot::default(),
            pair_done_tx,
        });
        (hub, pair_done_rx, wid_changed_rx)
    }

    /// Subscribe an additional consumer to the workspace-id-changed signal.
    ///
    /// `run_iroh` already holds one receiver (driving the gossip re-subscribe);
    /// the catch-up loop needs its own so it can clear its per-session `synced`
    /// dedup and re-dial under the adopted id. `broadcast` fans the one adoption
    /// event out to every receiver.
    pub(crate) fn subscribe_wid_changed(&self) -> tokio::sync::broadcast::Receiver<WorkspaceId> {
        self.wid_changed_tx.subscribe()
    }

    /// Snapshot the host's workspace id to advertise in its pairing payload.
    fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            .clone()
    }

    /// Adopt a remote (host's) workspace id: persist it to
    /// `<root>/.outl/workspace-id` and update the live shared handle, so this
    /// device now syncs as the host's workspace. Called by the JOINER after the
    /// handshake. Best-effort on the file write — the in-memory handle is updated
    /// regardless so the immediate post-pair sync uses the adopted id.
    fn adopt_workspace_id(&self, remote: WorkspaceId) {
        // Cheap skip if already adopted (avoids a redundant disk write).
        if *self
            .workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            == remote
        {
            return;
        }
        // Persist FIRST, then flip the in-memory handle. A half-adopted state
        // (memory on the host's id, disk still on the old one) would silently
        // split the workspace if the process died before the next write: the
        // next start reads the stale id from disk and never converges with the
        // host. So if the write fails we do NOT adopt — the pair just doesn't
        // take this time and the user retries, which is safe. Either both the
        // disk and memory move to the host's id, or neither does.
        if let Err(e) = remote.write(&self.workspace_root) {
            warn!("pairing: persist adopted workspace id failed, not adopting: {e}");
            return;
        }
        {
            let mut guard = self
                .workspace_id
                .write()
                .expect("workspace id rwlock poisoned");
            info!(adopted = %remote, "pairing: joiner adopted host's workspace id");
            *guard = remote.clone();
        }
        // Signal `run_iroh` so the gossip task re-subscribes to the new topic and
        // the catch-up loop clears its `synced` dedup and re-dials under the
        // adopted id. A send error means no receivers (transport shutting down) —
        // nothing to re-subscribe, so it's ignored. The in-memory handle is
        // already updated above, so direct delta-sync uses the adopted id
        // regardless of whether this signal is delivered.
        let _ = self.wid_changed_tx.send(remote);
    }

    /// Arm hosting and return the ticket the joiner needs, plus the receiver
    /// that resolves once a device completes the handshake.
    ///
    /// Replaces any prior arm (last-armer-wins): re-opening the pairing screen
    /// supersedes a stale session whose joiner never showed up.
    async fn arm_host(
        self: &Arc<Self>,
        alias: Option<String>,
    ) -> Result<(String, oneshot::Receiver<PeerEntry>, ArmGeneration)> {
        // The live endpoint is already online (the sync transport has been
        // running), but snapshot a ready addr anyway so the ticket carries the
        // current relay + direct addrs.
        let addr = ready_addr(&self.endpoint).await;
        let (ticket, secret) = mint_ticket(&addr).context("mint pairing ticket")?;

        let (result_tx, result_rx) = oneshot::channel();
        let gen = self.arm.arm(HostArm {
            alias,
            secret,
            result_tx,
        });
        Ok((ticket, result_rx, gen))
    }

    /// What an inbound connection needs to run the handshake, **without**
    /// disarming.
    ///
    /// Taking the arm up front meant any inbound dial consumed it: a stranger,
    /// a stale probe, or a joiner whose handshake failed would leave the user
    /// staring at a pairing screen that no longer accepted anyone, with nothing
    /// saying so. With a proof to check, that stopped being a rare accident and
    /// became a one-packet denial of the pairing flow (issue #159). The arm is
    /// now consumed only by a handshake that succeeded.
    fn arm_snapshot(&self) -> Option<(Option<String>, PairingSecret, ArmGeneration)> {
        self.arm.snapshot()
    }

    /// Consume the arm after a successful handshake, yielding the sender that
    /// resolves the awaiting `pair_host` future.
    ///
    /// Returns `None` if the arm vanished mid-handshake (the user closed the
    /// pairing screen, or re-armed). The peer is persisted either way — losing
    /// the notification is cosmetic, losing the pairing would not be.
    fn complete_arm(&self, gen: ArmGeneration) -> Option<HostArm> {
        self.arm.complete(gen)
    }

    /// Persist a freshly paired peer and signal an immediate sync against it.
    fn persist_and_kick(&self, entry: PeerEntry) -> Result<()> {
        let mut store = PeersStore::load_or_default(&self.peers_path)?;
        store.add(entry.clone())?;
        // Best-effort immediate sync: if the addr won't resolve we still
        // persisted, so the 8s catch-up loop picks it up next tick.
        if let Ok(addr) = entry.iroh_endpoint_addr() {
            let _ = self.pair_done_tx.send(addr);
        }
        Ok(())
    }
}

/// Router handler for the host (accept) side of pairing on the live endpoint.
#[derive(Clone)]
pub(crate) struct PairingProtocolHandler {
    pub(crate) hub: Arc<PairingHub>,
}

impl std::fmt::Debug for PairingProtocolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingProtocolHandler").finish()
    }
}

impl iroh::protocol::ProtocolHandler for PairingProtocolHandler {
    async fn accept(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        // Only accept while the user has armed hosting. An unarmed inbound
        // pairing dial is rejected (a stranger can't pair us in the background).
        let Some((alias, secret, gen)) = self.hub.arm_snapshot() else {
            warn!("inbound pairing connection while not armed; rejecting");
            conn.close(1u32.into(), b"not-armed");
            return Ok(());
        };

        if let Err(e) = self.serve(&conn, alias, &secret, gen).await {
            // Stay armed. The failure may be the attacker, or a first attempt
            // by the real device; either way the user's pairing screen must
            // keep working. Reporting the error to the router is still right —
            // it closes this connection — but it must not disarm.
            warn!("pairing serve failed (still armed): {e:#}");
            conn.close(2u32.into(), b"pairing-failed");
            return Err(iroh::protocol::AcceptError::from_boxed(e.into()));
        }
        Ok(())
    }
}

impl PairingProtocolHandler {
    async fn serve(
        &self,
        conn: &iroh::endpoint::Connection,
        alias: Option<String>,
        secret: &PairingSecret,
        gen: ArmGeneration,
    ) -> Result<()> {
        // Snapshot our ready addr so the joiner stores a reachable host.
        let our_addr = ready_addr(&self.hub.endpoint).await;
        // We are the HOST: advertise our workspace id (the joiner adopts it) but
        // keep our own — the host never adopts the joiner's id.
        let our_wid = self.hub.workspace_id();
        let (entry, _joiner_wid) = accept_host_handshake(
            conn,
            &self.hub.identity,
            &our_addr,
            alias,
            Some(&our_wid),
            secret,
        )
        .await?;

        self.hub.persist_and_kick(entry.clone())?;

        // Only now disarm: the handshake passed both checks. Deliver the result
        // to the awaiting `pair_host` future. A dropped receiver (the GUI gave
        // up waiting) is fine — the peer is persisted.
        if let Some(arm) = self.hub.complete_arm(gen) {
            let _ = arm.result_tx.send(entry);
        }

        // The host replied LAST; wait for the joiner to close so its read of our
        // payload isn't truncated. The live endpoint stays up regardless.
        conn.closed().await;
        Ok(())
    }
}

/// Drain the pair-completed channel, dialing each freshly paired peer once for
/// an immediate `delta_sync`. Reuses the transport's append lock + in-flight
/// guard + health map so an immediate sync behaves exactly like a catch-up dial.
///
/// Spawned by [`crate::engine::run_iroh`]; returns when the sender drops
/// (transport shutdown).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn drain_pair_completions(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<iroh::EndpointAddr>,
    conns: crate::peer_conn::PeerConnections,
    workspace_root: std::path::PathBuf,
    workspace_id: SharedWorkspaceId,
    actor: outl_core::id::ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    health: crate::health::PeerHealthMap,
    append_lock: crate::engine::AppendLock,
    in_flight: crate::engine::InFlightPeers,
    progress: crate::progress::ProgressSink,
) {
    while let Some(addr) = rx.recv().await {
        let nid = addr.id;
        let Some(_guard) = crate::engine::try_acquire_in_flight(&in_flight, nid) else {
            continue;
        };
        let started = std::time::Instant::now();
        info!(peer = %nid.fmt_short(), "pairing: immediate sync to freshly paired peer");
        // Read the LIVE id: if this device just adopted the host's id during the
        // handshake, the immediate post-pair sync must use the adopted value so
        // the responder accepts it.
        let wid_snapshot = workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            .clone();
        match crate::engine::delta_sync(
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
            Ok(()) => health.record_success(nid, started),
            Err(e) => {
                warn!("pairing: immediate sync to {} failed: {e}", nid.fmt_short());
                health.record_failure(nid);
            }
        }

        // Phase 2 snapshot sync: also pull the peer's materialized snapshot so a
        // freshly-paired device boots from settled state instead of replaying the
        // (potentially 76 MB) op log the delta-sync just streamed. A separate
        // connection on `SNAPSHOT_ALPN`; kept sequential (still inside this
        // peer's in-flight guard) after the delta-sync. Best-effort: an absent
        // peer snapshot or any transfer error is harmless — the op log stays the
        // source of truth and boot falls back to full replay. On success it fires
        // its own `peer_ready_tx` so the reload adopts the cached snapshot.
        match crate::engine_snapshot::pull_snapshot_from_peer(
            conns.endpoint(),
            addr.clone(),
            &workspace_root,
            &peer_ready_tx,
            &progress,
        )
        .await
        {
            Ok(true) => info!("pairing: pulled snapshot from {}", nid.fmt_short()),
            Ok(false) => {}
            Err(e) => warn!(
                "pairing: snapshot pull from {} failed: {e}",
                nid.fmt_short()
            ),
        }

        // Asset sync: pull the peer's binary assets (uploaded PDFs / images) that
        // never entered the op log. Another separate connection on `ASSET_ALPN`,
        // still sequential inside this peer's in-flight guard. Best-effort: an
        // absent / unreadable / hash-mismatched asset is skipped — a link to a
        // not-yet-transferred asset just renders dead until the next pull. See
        // `crate::engine_assets`.
        match crate::engine_assets::pull_assets_from_peer(
            conns.endpoint(),
            addr,
            &workspace_root,
            &progress,
        )
        .await
        {
            Ok(n) if n > 0 => info!("pairing: pulled {n} assets from {}", nid.fmt_short()),
            Ok(_) => {}
            Err(e) => warn!("pairing: asset pull from {} failed: {e}", nid.fmt_short()),
        }
    }
}

/// Run the host side of GUI pairing on the live endpoint.
///
/// `on_ticket` fires the moment the ticket is known (so the frontend can render
/// the QR), then the future resolves once a device completes the handshake or
/// [`HOST_ACCEPT_TIMEOUT`] elapses.
///
/// Every endpoint op runs on the transport's runtime (`hub.runtime`), spawned
/// from the GUI's runtime: `pair_host` is called from a Tauri command, but the
/// endpoint was bound on the transport's own runtime, so its `connect` /
/// `accept` futures must be driven there.
pub(crate) async fn pair_host_on_hub<F>(
    hub: Arc<PairingHub>,
    alias: Option<String>,
    on_ticket: F,
) -> Result<PeerEntry>
where
    F: FnOnce(&str) + Send + 'static,
{
    // Arm + mint the ticket on the transport's runtime (touches the endpoint).
    let arm_hub = hub.clone();
    let (ticket, result_rx, gen) = hub
        .runtime
        .spawn(async move { arm_hub.arm_host(alias).await })
        .await
        .context("join arm-host task")??;

    on_ticket(&ticket);

    // Await the handshake result (filled by the router handler) on the
    // transport's runtime too, so the timeout timer and the channel live on the
    // runtime driving the endpoint.
    let wait_hub = hub.clone();
    hub.runtime
        .spawn(async move {
            match tokio::time::timeout(HOST_ACCEPT_TIMEOUT, result_rx).await {
                Ok(Ok(entry)) => Ok(entry),
                Ok(Err(_)) => Err(anyhow!(
                    "pairing host channel closed before a device paired"
                )),
                Err(_) => {
                    // Disarm so a later inbound dial doesn't complete against a
                    // dropped receiver — but only **our** session.
                    //
                    // This task lives on the transport's runtime and outlives
                    // the future that spawned it, so by the time it fires the
                    // user may have closed and re-opened the pairing screen.
                    // Disarming unconditionally would take the arm behind the
                    // ticket now on screen, which then answers `not-armed` and
                    // reads as pairing being broken.
                    let _ = wait_hub.complete_arm(gen);
                    Err(anyhow!("timed out waiting for the other device to connect"))
                }
            }
        })
        .await
        .context("join host-wait task")?
}

/// Run the joiner side of GUI pairing on the live endpoint.
///
/// Spawned onto the transport's runtime for the same reason as
/// [`pair_host_on_hub`] — the endpoint must be dialed from the runtime that
/// bound it.
pub(crate) async fn pair_join_on_hub(
    hub: Arc<PairingHub>,
    ticket: String,
    alias: Option<String>,
) -> Result<PeerEntry> {
    let join_hub = hub.clone();
    hub.runtime
        .spawn(async move {
            let our_addr = ready_addr(&join_hub.endpoint).await;
            // We are the JOINER: advertise our current id, then ADOPT the host's.
            let our_wid = join_hub.workspace_id();
            let (entry, host_wid) = run_join_handshake(
                &join_hub.endpoint,
                &join_hub.identity,
                &ticket,
                &our_addr,
                alias,
                Some(&our_wid),
            )
            .await?;
            // Adopt the host's workspace id BEFORE the immediate post-pair sync
            // fires (persist_and_kick), so that sync uses the shared id and the
            // host's responder accepts it.
            if let Some(host_wid) = host_wid {
                join_hub.adopt_workspace_id(host_wid);
            }
            join_hub.persist_and_kick(entry.clone())?;
            Ok::<PeerEntry, anyhow::Error>(entry)
        })
        .await
        .context("join pair-join task")?
}

#[cfg(test)]
mod arm_slot_tests {
    use super::*;

    fn an_arm() -> (HostArm, oneshot::Receiver<PeerEntry>) {
        let (result_tx, rx) = oneshot::channel();
        (
            HostArm {
                alias: Some("desktop".into()),
                secret: PairingSecret::generate(),
                result_tx,
            },
            rx,
        )
    }

    /// The GUI half of issue #159's re-arm: reading the arm to run a handshake
    /// must not disarm, no matter how many connections arrive.
    ///
    /// If this regresses, the pairing screen stops accepting after the first
    /// inbound dial — and because a refused stranger is now an *expected*
    /// outcome rather than a rare accident, anyone who can reach the device can
    /// end the user's pairing session with one packet.
    #[test]
    fn reading_the_arm_never_disarms_it() {
        let slot = ArmSlot::default();
        let (arm, _rx) = an_arm();
        slot.arm(arm);

        for attempt in 1..=5 {
            assert!(
                slot.snapshot().is_some(),
                "arm vanished after {attempt} inbound connection(s)",
            );
            assert!(slot.is_armed(), "still armed after {attempt} read(s)");
        }
    }

    /// The other side of the same rule: a handshake that passed consumes it,
    /// so a second joiner cannot pair against a session already spent.
    #[test]
    fn completing_the_handshake_disarms() {
        let slot = ArmSlot::default();
        let (arm, _rx) = an_arm();
        let gen = slot.arm(arm);

        assert!(
            slot.complete(gen).is_some(),
            "the first completion takes it"
        );
        assert!(!slot.is_armed(), "must be disarmed after a completion");
        assert!(
            slot.complete(gen).is_none(),
            "a second completion must find nothing",
        );
        assert!(
            slot.snapshot().is_none(),
            "an inbound connection after completion must find no arm",
        );
    }

    /// A refused joiner is exactly a `snapshot` with no `complete`, and the
    /// real device must still get through afterwards.
    #[test]
    fn a_refused_joiner_leaves_the_next_one_able_to_pair() {
        let slot = ArmSlot::default();
        let (arm, _rx) = an_arm();
        slot.arm(arm);

        // Attacker: reads the arm, fails the proof, never completes.
        let stolen = slot.snapshot();
        assert!(
            stolen.is_some(),
            "the handler reads the arm to run a handshake"
        );

        // Real device: still finds an arm, and completes.
        let (_, _, gen) = slot.snapshot().expect("the arm survived the refusal");
        assert!(
            slot.complete(gen).is_some(),
            "the invited device must still be able to pair",
        );
    }

    /// The stale-timeout case, and the reason `complete` takes a generation.
    ///
    /// `pair_host_on_hub` spawns its host-wait onto the transport's runtime,
    /// so a wait from a pairing screen the user already closed is still alive
    /// and still fires. Before the generation it called `complete()` bare and
    /// took whatever was armed — including the session behind the ticket now on
    /// screen, which then answered `not-armed` and read as broken pairing.
    #[test]
    fn a_stale_timeout_cannot_disarm_a_newer_session() {
        let slot = ArmSlot::default();

        let (first, _rx1) = an_arm();
        let stale_gen = slot.arm(first);

        // User closes and re-opens the pairing screen.
        let (second, _rx2) = an_arm();
        let live_gen = slot.arm(second);
        assert_ne!(stale_gen, live_gen);

        // The old wait times out now and tries to disarm.
        assert!(
            slot.complete(stale_gen).is_none(),
            "a superseded session must not consume the current arm",
        );
        assert!(
            slot.is_armed(),
            "the ticket on screen must still be able to pair",
        );

        // And the current session still completes normally.
        assert!(slot.complete(live_gen).is_some());
    }

    /// Re-opening the pairing screen supersedes a stale session rather than
    /// stacking, so the ticket on screen is always the one that will verify.
    #[test]
    fn re_arming_replaces_the_previous_session() {
        let slot = ArmSlot::default();
        let (first, _rx1) = an_arm();
        let first_secret = first.secret.clone();
        slot.arm(first);

        let (second, _rx2) = an_arm();
        let second_secret = second.secret.clone();
        slot.arm(second);

        // Compare the secrets through the proof they produce for one fixed
        // device — the only observable a `PairingSecret` exposes, by design.
        let probe = iroh::SecretKey::generate().public();
        let (_, live, _) = slot.snapshot().expect("still armed");
        assert_eq!(
            live.proof_for(probe),
            second_secret.proof_for(probe),
            "the live secret must be the most recent arm's",
        );
        assert_ne!(
            live.proof_for(probe),
            first_secret.proof_for(probe),
            "a joiner holding the superseded ticket must no longer verify",
        );
    }
}
