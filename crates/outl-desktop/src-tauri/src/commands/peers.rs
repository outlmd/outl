//! Tauri commands for peer/device management.
//!
//! List / remove / status / force-sync are thin wrappers over
//! `outl_tauri_shared::commands::peers`. The **pairing** commands stay
//! desktop-local: they need the concrete `IrohSyncTransport` from
//! `AppState::iroh_pairing` (pairing isn't a `SyncTransport` trait
//! concern) and their reply shape (`PairedPeerDto` + the early
//! `peer-pairing-ticket` event) is the desktop's wire contract — the
//! mobile client resolves the raw ticket string instead.
//!
//! ## Pairing without an endpoint of our own
//!
//! `AppState::iroh_pairing` is empty whenever this process didn't get the
//! device endpoint: P2P switched off, another local outl process (usually
//! `outl mcp serve`, launched at login) won the lease, the lease could not be
//! arbitrated, or the transport failed to build. `iroh_sync::NoEndpoint` says
//! which, and only the first refuses to pair. Refusing to pair in the others
//! would leave a user who runs two outl processes unable to add a device, so both
//! pairing commands fall back to the CLI's one-shot helpers
//! (`outl_sync_iroh::host_pairing` / `join_pairing`), which bind their own
//! endpoint and close it before returning. The cost of that fallback, and why
//! it is confined to these two commands, is spelled out on
//! [`outl_peer_pair_host`].

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::iroh_sync::{no_endpoint_reason, NoEndpoint};
use crate::state::AppState;
use outl_tauri_shared::commands::peers::{self as shared, PeerDto, PeerStatusDto};
use outl_tauri_shared::AppHost;

/// List all paired devices.
#[tauri::command]
pub fn outl_peer_list(state: State<'_, AppState>) -> Result<Vec<PeerDto>, String> {
    shared::peer_list(state.inner())
}

/// Remove a peer by node_id prefix.
#[tauri::command]
pub fn outl_peer_remove(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    shared::peer_remove(state.inner(), id)
}

/// Reachability for each paired peer, read from the **running** iroh
/// transport's own dial outcomes — see the shared body for why a fresh
/// probe endpoint is never bound.
#[tauri::command]
pub fn outl_peer_status(state: State<'_, AppState>) -> Result<Vec<PeerStatusDto>, String> {
    shared::peer_status(state.inner())
}

/// Force an immediate P2P sync pass against every paired peer — the
/// trigger behind the Sync panel's Refresh.
///
/// When another local outl process holds the device endpoint this window has
/// no transport to force, and the shared body would return `Ok(())` after
/// doing nothing. That silence is the whole defect: the user presses Refresh,
/// the dot stays orange, and nothing anywhere says why. So this one case is
/// reported as an error, which the Sync panel already surfaces via
/// `appState.lastError`. `[sync] transport = "file"` stays a quiet no-op —
/// that is the user's own choice, not a degraded state.
///
/// Before reporting that, it contends for the endpoint once more; see
/// [`retry_endpoint`] for why Refresh is the right place to re-run the
/// election.
#[tauri::command]
pub fn outl_sync_now(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // Read and release: `retry_endpoint` writes the same slot.
    let wired = state.iroh_transport.lock().is_some();
    if !wired {
        retry_endpoint(&app, state.inner());
        if let Some(notice) = degraded_endpoint_notice() {
            return Err(notice);
        }
    }
    shared::sync_now(state.inner())
}

/// Contend for the device endpoint one more time, when this window has none.
///
/// The recorded reason is a snapshot of a single moment, usually login. The
/// process that won the lease then (typically `outl mcp serve`) can exit at any
/// point afterwards, and nothing in this process notices: the endpoint stays
/// free, this window keeps no transport, and Refresh keeps printing "another
/// process holds it" about a process that is gone. Refresh is an explicit
/// user request and one `flock` attempt is cheap, so it is where the election
/// is re-run.
///
/// Two states are left alone. `P2pDisabled` is a setting, not a race, and
/// re-contending there would bind an endpoint the user switched off. A `None`
/// reason with no transport means the boot opener has not wired yet, and a
/// second concurrent pass would race it for the same lease.
fn retry_endpoint(app: &AppHandle, state: &AppState) {
    match no_endpoint_reason() {
        Some(NoEndpoint::HeldByAnotherProcess) | Some(NoEndpoint::Unavailable(_)) => {}
        None | Some(NoEndpoint::P2pDisabled) => return,
    }
    let Some(root) = state.storage_root.lock().clone() else {
        return;
    };
    crate::iroh_sync::wire_iroh_transport(
        &state.iroh_transport,
        &state.iroh_pairing,
        root,
        state.hlc.actor(),
        app.clone(),
    );
}

/// The Refresh error for a window with no transport, or `None` when the state
/// needs no explanation (`transport = "file"`, or we hold the endpoint).
///
/// One place so the wording can't drift across surfaces, and one place so the
/// "another process has it" sentence is never printed for a state where no
/// such process exists.
fn degraded_endpoint_notice() -> Option<String> {
    match no_endpoint_reason()? {
        // The user's own choice is not a degraded state.
        NoEndpoint::P2pDisabled => None,
        NoEndpoint::HeldByAnotherProcess => Some(ENDPOINT_BUSY_NOTICE.to_string()),
        NoEndpoint::Unavailable(why) => Some(format!(
            "This window could not claim the device's P2P endpoint ({why}), so it \
             syncs through the shared ops/ folder instead. Edits still converge; \
             live peer status and Refresh stay unavailable."
        )),
    }
}

/// One wording for the lost-the-election state, so the Refresh error and any
/// later surface can't drift apart.
const ENDPOINT_BUSY_NOTICE: &str =
    "Another outl process on this device (usually `outl mcp serve`) \
     holds the P2P endpoint, so this window syncs through the shared ops/ \
     folder instead. Edits still converge; live peer status and Refresh stay \
     unavailable until that process exits. Press Refresh again once it has: \
     each press re-runs the election.";

/// Why pairing refuses when P2P is off, rather than quietly turning it on for
/// one handshake.
const P2P_DISABLED_NOTICE: &str =
    "P2P sync is turned off (Settings → Sync → transport is \"file\"). \
     Switch it to iroh to pair a device.";

/// Result of a completed pairing handshake — the peer that was added.
#[derive(serde::Serialize)]
pub struct PairedPeerDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub added_at: String,
}

impl From<outl_sync_iroh::PeerEntry> for PairedPeerDto {
    fn from(p: outl_sync_iroh::PeerEntry) -> Self {
        PairedPeerDto {
            node_id: p.node_id,
            alias: p.alias,
            added_at: p.added_at,
        }
    }
}

/// Everything the one-shot pairing helpers need when this process has no live
/// transport: the device identity, the workspace's `peers.json` path, and the
/// workspace root (the graph the pairing belongs to, and where `join_pairing`
/// writes the adopted `workspace-id`).
///
/// Same three inputs `outl peer pair` assembles in `outl-cli/src/main.rs`;
/// identity is per-**device** (`~/.outl/identity.key`) while the peer list is
/// per-**graph**, hence the migration call before resolving the path.
fn one_shot_pairing_inputs(
    state: &AppState,
) -> Result<(Arc<outl_sync_iroh::IrohIdentity>, PathBuf, PathBuf), String> {
    // Several reasons land here and only one of them forbids binding an
    // endpoint.
    //
    // P2P switched off (`[sync] transport = "file"`): refuse. Binding an
    // endpoint would override the setting the user chose, on the one path where
    // we know they are looking at the app. Say what to do instead of doing it
    // for them.
    //
    // Anything else (lost the election, an unarbitrable lease, a transport that
    // failed to build): pair over a one-shot endpoint. The user asked to add a
    // device and the alternative is being unable to. A cause that also breaks
    // the one-shot path, an unreadable identity say, surfaces its own error
    // below, which beats telling the user to switch on a setting that is
    // already on.
    let reason = no_endpoint_reason();
    if matches!(reason, Some(NoEndpoint::P2pDisabled)) {
        return Err(P2P_DISABLED_NOTICE.to_string());
    }
    tracing::info!(
        ?reason,
        "no endpoint of our own; pairing over a one-shot endpoint"
    );
    let root = AppHost::storage_root(state)?;
    let device_dir = outl_sync_iroh::default_device_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&device_dir).map_err(|e| e.to_string())?;
    let identity = outl_sync_iroh::IrohIdentity::load_or_generate(&device_dir.join("identity.key"))
        .map_err(|e| e.to_string())?;
    outl_sync_iroh::migrate_global_peers_if_absent(&root);
    let peers_path = outl_sync_iroh::workspace_peers_path(&root);
    Ok((Arc::new(identity), peers_path, root))
}

/// Host a pairing session: emit the ticket **early** (so the frontend can
/// render it / a QR while we wait), then block until the other device connects
/// and completes the handshake.
///
/// Mirrors the mobile/CLI design: the ticket is surfaced before the
/// command resolves via the `peer-pairing-ticket` event (`{ ticket }`),
/// and `peer-paired` (`PairedPeerDto`) fires once a peer is persisted to
/// the workspace's `.outl/peers.json`. The command resolves with the same
/// [`PairedPeerDto`] so a caller that prefers awaiting over listening
/// also gets the result.
///
/// `alias` is an optional human label advertised to the peer.
#[tauri::command]
pub async fn outl_peer_pair_host(
    app: AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
) -> Result<PairedPeerDto, String> {
    // `pair_host` / `host_pairing` both invoke `on_ticket` the moment the ticket
    // is known — before blocking on the inbound connection — so emitting from
    // there gets the ticket to the UI immediately. One closure for both paths so
    // the event contract can't diverge between them.
    let ticket_app = app.clone();
    let emit_ticket = move |ticket: &str| {
        if let Err(e) = ticket_app.emit(
            "peer-pairing-ticket",
            PairingTicketPayload {
                ticket: ticket.to_string(),
            },
        ) {
            tracing::warn!("emit peer-pairing-ticket: {e}");
        }
    };

    // Prefer the LIVE sync endpoint — that path never binds a second endpoint
    // with the device identity, so it cannot hijack our own relay route. See
    // `outl-sync-iroh/CLAUDE.md` → "One endpoint per identity, elected not
    // assigned".
    let live = state.iroh_pairing.lock().clone();
    let entry = match live {
        Some(transport) => transport
            .pair_host(alias, emit_ticket)
            .await
            .map_err(|e| e.to_string())?,
        // No endpoint of our own: either another local process won the lease or
        // P2P is off. Fall back to the CLI's one-shot helper, which binds its
        // own endpoint and closes it before returning.
        //
        // TRADE-OFF, accepted knowingly: for the seconds that handshake runs,
        // this endpoint takes the relay route away from whichever local process
        // holds the lease, so that process's sync stalls. It is acceptable
        // because pairing is rare, explicitly user-initiated, short, and the
        // holder recovers the route as soon as the one-shot endpoint closes —
        // whereas the alternative is a user who simply cannot add a device.
        // This is NOT a precedent: no other GUI path may bind an endpoint.
        None => {
            let (identity, peers_path, root) = one_shot_pairing_inputs(state.inner())?;
            outl_sync_iroh::host_pairing(identity, &peers_path, &root, alias, |ticket, _qr| {
                emit_ticket(ticket)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    };

    let dto: PairedPeerDto = entry.into();
    if let Err(e) = app.emit("peer-paired", &dto) {
        tracing::warn!("emit peer-paired: {e}");
    }
    Ok(dto)
}

/// Join a pairing session from a ticket string produced by a host's
/// [`outl_peer_pair_host`]. Dials over the **live sync endpoint** when this
/// process holds one (otherwise a one-shot endpoint, same trade-off as
/// [`outl_peer_pair_host`]), completes the handshake, persists the host to the
/// workspace's `.outl/peers.json`, and emits `peer-paired`.
#[tauri::command]
pub async fn outl_peer_pair_join(
    app: AppHandle,
    state: State<'_, AppState>,
    ticket: String,
    alias: Option<String>,
) -> Result<PairedPeerDto, String> {
    let live = state.iroh_pairing.lock().clone();
    let entry = match live {
        Some(transport) => transport
            .pair_join(ticket, alias)
            .await
            .map_err(|e| e.to_string())?,
        None => {
            let (identity, peers_path, root) = one_shot_pairing_inputs(state.inner())?;
            // The joiner also ADOPTS the host's workspace id here (persisted to
            // `<root>/.outl/workspace-id`), which is what keeps later syncs from
            // being refused as `workspace-mismatch` — issue #197. There is no
            // live transport holding a workspace-id handle to refresh, so the
            // file write is the whole adoption.
            let (entry, adopted) =
                outl_sync_iroh::join_pairing(identity, &ticket, &peers_path, &root, alias)
                    .await
                    .map_err(|e| e.to_string())?;
            tracing::info!(?adopted, "one-shot pairing joined");
            entry
        }
    };

    let dto: PairedPeerDto = entry.into();
    if let Err(e) = app.emit("peer-paired", &dto) {
        tracing::warn!("emit peer-paired: {e}");
    }
    Ok(dto)
}

/// Payload for the early `peer-pairing-ticket` event the host emits
/// before the handshake completes.
#[derive(serde::Serialize, Clone)]
struct PairingTicketPayload {
    ticket: String,
}
