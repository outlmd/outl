//! Tauri commands for peer/device management + pairing.
//!
//! List / remove / status / force-sync are thin wrappers over
//! `outl_tauri_shared::commands::peers` — the only commands that bypass
//! the workspace lock (peer pairing is sync-transport state, not
//! workspace state).
//!
//! The **pairing** commands stay mobile-local: they clone the concrete
//! `IrohSyncTransport` out of `AppState::iroh`, and their wire contract
//! diverges from the desktop's on purpose — `outl_peer_pair_host`
//! resolves with the raw ticket `String` (the QR is rendered client-side
//! from it) while the desktop resolves with the paired-peer DTO and
//! surfaces the ticket via an event.
//!
//! ## Where the iroh files live (mobile-specific)
//!
//! The device **identity** (`identity.key`) is per-install and lives in
//! the Tauri **app local data dir** ([`crate::iroh_sync::iroh_dir`]) — iOS
//! has no meaningful home directory, so that is the per-device analogue of
//! `~/.outl/`. The **peer list** is per-GRAPH, so it lives at
//! `<workspace_root>/.outl/peers.json`; the shared body resolves it from
//! `AppState::storage_root`, the same root the live transport in
//! `iroh_sync::wire_iroh_transport` reads, so a freshly paired device
//! shows up in `outl_peer_list` and starts syncing after the next launch
//! without a second source of truth.

use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;
use outl_tauri_shared::commands::peers::{self as shared, PeerDto, PeerStatusDto};

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
/// trigger behind the refresh button / pull-to-refresh.
#[tauri::command]
pub fn outl_sync_now(state: State<'_, AppState>) -> Result<(), String> {
    shared::sync_now(state.inner())
}

/// Host one pairing session and return the ticket **immediately**.
///
/// ## Why the ticket comes back before pairing finishes
///
/// `pair_host` binds over the live endpoint, surfaces the ticket via an
/// `on_ticket` callback, and *then blocks* waiting for the other device
/// to connect (up to a 120 s internal timeout). The frontend needs the
/// ticket string up front to render the QR code while the user walks it
/// over to the second device — it can't wait for the whole handshake.
///
/// So this command:
///
/// 1. Spawns `pair_host` on a Tokio task.
/// 2. The `on_ticket` callback sends the ticket through a oneshot.
/// 3. The command `await`s that oneshot and returns the ticket as soon as
///    the endpoint is armed — long before a peer connects.
/// 4. The spawned task keeps running. When a device pairs (or it times
///    out), it emits `peer-paired` with the new [`PeerDto`] on success so
///    the frontend can refresh its device list. On failure it emits
///    `peer-pair-failed` with the error string.
///
/// The peer is persisted to the same `peers.json` the live transport
/// syncs against, so it takes effect on the next transport start.
#[tauri::command]
pub async fn outl_peer_pair_host(
    app: AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
) -> Result<String, String> {
    // Reuse the LIVE sync endpoint for pairing — never bind a second endpoint
    // with the device identity. A second endpoint hijacks the relay route from
    // the running transport and silently kills sync (the "Another endpoint
    // connected with the same endpoint id" relay error). See
    // `outl-sync-iroh/CLAUDE.md` → "One endpoint per identity, elected not
    // assigned".
    let transport = state
        .iroh
        .clone()
        .ok_or_else(|| "iroh transport not running; cannot pair".to_string())?;

    // A capacity-1 mpsc stands in for a oneshot: `tauri::async_runtime`
    // re-exports tokio's mpsc but not its oneshot. The `on_ticket`
    // closure is a *sync* `FnOnce`, so it uses the non-blocking
    // `try_send` (capacity 1 guarantees it never returns `Full` for the
    // single ticket).
    let (ticket_tx, mut ticket_rx) = tauri::async_runtime::channel::<String>(1);
    let app_for_task = app.clone();

    // The accept side outlives this command — it runs until a peer pairs
    // or the host-accept timeout fires inside the transport's `pair_host`.
    tauri::async_runtime::spawn(async move {
        let result = transport
            .pair_host(alias, move |ticket: &str| {
                // Hand the ticket back to the awaiting command; the QR is
                // rendered client-side from this string.
                let _ = ticket_tx.try_send(ticket.to_string());
            })
            .await;

        match result {
            Ok(entry) => {
                if let Err(e) = app_for_task.emit("peer-paired", PeerDto::from(entry)) {
                    tracing::warn!("emit peer-paired: {e}");
                }
            }
            Err(e) => {
                if let Err(emit_err) = app_for_task.emit("peer-pair-failed", e.to_string()) {
                    tracing::warn!("emit peer-pair-failed: {emit_err}");
                }
            }
        }
    });

    // Resolve as soon as the ticket is known. `recv()` yielding `None` means
    // the sender was dropped before a ticket arrived (the task errored before
    // arming) — surface that as the command error.
    ticket_rx
        .recv()
        .await
        .ok_or_else(|| "pairing host failed before producing a ticket".to_string())
}

/// Join a pairing session with a ticket from the hosting device.
///
/// `pair_join` connects over the live endpoint, runs the handshake,
/// persists the remote peer, and returns the stored entry — so this is a
/// plain async command that maps the result to a [`PeerDto`]. No event
/// needed: the caller gets the new peer back directly.
#[tauri::command]
pub async fn outl_peer_pair_join(
    state: State<'_, AppState>,
    ticket: String,
    alias: Option<String>,
) -> Result<PeerDto, String> {
    // Dial out over the LIVE sync endpoint, not a fresh one (see
    // `outl_peer_pair_host` for why a second endpoint is fatal).
    let transport = state
        .iroh
        .clone()
        .ok_or_else(|| "iroh transport not running; cannot pair".to_string())?;
    let entry = transport
        .pair_join(ticket, alias)
        .await
        .map_err(|e| e.to_string())?;
    Ok(PeerDto::from(entry))
}
