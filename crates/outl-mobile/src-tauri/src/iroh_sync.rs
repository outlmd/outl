//! iroh P2P sync wiring for the mobile client.
//!
//! The build + start + reload-bridge machinery lives in
//! `outl_tauri_shared::iroh_sync`; this module keeps what is genuinely
//! mobile:
//!
//! - **Where the device identity lives.** Unlike the desktop / TUI
//!   (which use `~/.outl/`), iOS has no meaningful home directory — the
//!   sandbox `$HOME` is per-install and opaque. We resolve
//!   `identity.key` from the Tauri **app local data dir** so it sits
//!   next to the persisted `actor` ULID and survives across launches.
//!   [`iroh_dir`] is the single source of that per-device path. The
//!   identity is per-DEVICE (one node id per install), never per-graph.
//!
//! - **Which event signals a reload.** The transport's "peer ops landed"
//!   signal bridges to `workspace-ready` — the same event the background
//!   opener fires, which the frontend already listens for to reload the
//!   current view (iroh has no native watcher, so it reuses the Tauri
//!   event).
//!
//! - **The iOS BGProcessingTask hook.** The live transport is registered
//!   into the `bg_sync` global so a background window can drive a forced
//!   sync pass.
//!
//! - **Resilience.** Any failure binding the identity / peer store logs
//!   and returns `None`. The app keeps running without sync rather than
//!   crashing startup; a later relaunch retries.

use std::path::PathBuf;

use outl_core::id::ActorId;
use outl_sync_iroh::{IrohSyncTransport, TransportOutcome};
use outl_tauri_shared::iroh_sync::start_with_reload_bridge;
use tauri::Manager;
use tracing::{info, warn};

/// Directory holding the device's iroh identity.
///
/// Lives under the Tauri app local data dir (the same place the `actor`
/// ULID is persisted) — never the iCloud container. The device identity
/// is per-install and must not sync between devices: two devices sharing
/// one `identity.key` would advertise the same node id and collide.
pub(crate) fn iroh_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("app local data dir: {e}"))?;
    Ok(dir)
}

pub(crate) fn identity_path(dir: &std::path::Path) -> PathBuf {
    dir.join("identity.key")
}

/// Build + start the iroh transport when the config asks for it.
///
/// Returns the live [`IrohSyncTransport`] so the caller can stash it in
/// `AppState` (keeping its background tokio runtime alive). Returns
/// `None` when:
///
/// - the config selects the file transport (`transport = "file"`),
/// - another process on this device already holds the endpoint (never on iOS
///   today — one app, one sandbox — but the decision has one owner), or
/// - the app local data dir / identity / peer store can't be resolved
///   (logged, never fatal).
pub(crate) fn wire_iroh_transport(
    app: &tauri::AppHandle,
    workspace_root: PathBuf,
    actor: ActorId,
) -> Option<IrohSyncTransport> {
    let dir = match iroh_dir(app) {
        Ok(d) => d,
        Err(e) => {
            warn!("iroh disabled: {e}");
            return None;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("iroh disabled: create {}: {e}", dir.display());
        return None;
    }

    let transport = match outl_sync_iroh::build_transport(&identity_path(&dir), &workspace_root) {
        Ok(TransportOutcome::Ready(t)) => t,
        Ok(TransportOutcome::EndpointBusy) => {
            info!("another local outl process holds the iroh endpoint; iroh disabled here");
            return None;
        }
        Ok(TransportOutcome::Disabled) => {
            info!("sync transport = file; iroh disabled by config");
            return None;
        }
        Err(e) => {
            warn!("iroh disabled: {e}");
            return None;
        }
    };

    // Bridge each "peer ops landed" signal to the `workspace-ready`
    // Tauri event so the frontend reloads.
    start_with_reload_bridge(
        &transport,
        workspace_root.clone(),
        actor,
        app.clone(),
        "workspace-ready",
    );

    // Expose the live transport + workspace root to the iOS background-task
    // FFIs: a background window drives a forced sync pass, and the peer-count
    // gate reads `<root>/.outl/peers.json` to decide whether scheduling a
    // window is worth it at all (see `bg_sync`).
    crate::bg_sync::register(&transport, workspace_root);

    info!("iroh sync transport started");
    Some(transport)
}
