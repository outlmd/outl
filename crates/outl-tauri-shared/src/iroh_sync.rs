//! iroh P2P transport wiring shared by the GUI clients.
//!
//! Whether a transport may be built at all — the configured transport kind, the
//! device endpoint lease, the identity / peers / relay recipe — belongs to
//! `outl_sync_iroh::build_transport`, the one owner every client calls.
//! What is left here is the half that is genuinely Tauri: turning the
//! transport's two internal signals into app events.
//!
//! The clients still diverge on *where the identity lives* (desktop:
//! `~/.outl/identity.key`, shared with the CLI/TUI; mobile: the Tauri app
//! local data dir — iOS has no meaningful home) and on *which Tauri event
//! signals a reload* (`peer-ops-changed` on desktop, `workspace-ready` on
//! mobile). Both are parameters at the call site.

use std::sync::mpsc;

use outl_actions::{SyncProgress, SyncTransport};
use outl_core::id::ActorId;
use outl_sync_iroh::IrohSyncTransport;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

/// The Tauri event carrying [`SyncProgress`] updates to the frontend's
/// pairing-screen progress feed. Same name on both clients (unlike the
/// reload event, which differs) — the payload is the serialized
/// `SyncProgress` enum (tagged by `phase`).
pub const SYNC_PROGRESS_EVENT: &str = "sync-progress";

/// Start `transport` and bridge its two internal signals to Tauri events.
///
/// 1. The **reload** signal (`peer_ready`, `()`): fired whenever peer ops
///    land in local `ops/`; each becomes `app.emit(reload_event, ())` so
///    the frontend's existing reload path is reused verbatim.
/// 2. The **progress** signal ([`SyncProgress`]): fired as a sync pass
///    advances (connecting, snapshot bytes, ops counts); each becomes
///    `app.emit(SYNC_PROGRESS_EVENT, progress)` for the pairing-screen
///    progress feed. Purely cosmetic — a dropped update never affects
///    correctness (the reload signal is the load-bearing one).
///
/// Both bridge threads end cleanly when the transport drops its senders
/// (shutdown). The progress sink is registered **before** `start()`, per
/// the [`SyncTransport::set_progress_sink`] contract.
pub fn start_with_reload_bridge(
    transport: &IrohSyncTransport,
    workspace_root: std::path::PathBuf,
    actor: ActorId,
    app: AppHandle,
    reload_event: &'static str,
) {
    // Progress sink first (contract: register before start), then the reload
    // signal, then start.
    let (progress_tx, progress_rx) = mpsc::channel::<SyncProgress>();
    transport.set_progress_sink(progress_tx);

    let (peer_ready_tx, peer_ready_rx) = mpsc::channel::<()>();
    transport.start(workspace_root, actor, peer_ready_tx);

    let reload_app = app.clone();
    std::thread::Builder::new()
        .name("outl-iroh-bridge".into())
        .spawn(move || {
            // Recv blocks until the transport signals or the sender is
            // dropped (transport shut down). Either way the loop ends
            // cleanly when the channel disconnects.
            while peer_ready_rx.recv().is_ok() {
                if let Err(e) = reload_app.emit(reload_event, ()) {
                    warn!("emit {reload_event} (iroh): {e}");
                }
            }
            info!("iroh peer-ready bridge ended");
        })
        .expect("spawning the iroh peer-ready bridge thread should not fail");

    std::thread::Builder::new()
        .name("outl-iroh-progress".into())
        .spawn(move || {
            while let Ok(progress) = progress_rx.recv() {
                if let Err(e) = app.emit(SYNC_PROGRESS_EVENT, &progress) {
                    warn!("emit {SYNC_PROGRESS_EVENT} (iroh): {e}");
                }
            }
            info!("iroh progress bridge ended");
        })
        .expect("spawning the iroh progress bridge thread should not fail");
}
