//! iroh P2P transport wiring for the desktop client.
//!
//! The build + start + reload-bridge machinery lives in
//! `outl_tauri_shared::iroh_sync`; this module keeps what is genuinely
//! desktop:
//!
//! - **Where the identity lives.** `~/.outl/identity.key` — the same
//!   per-device path the CLI / TUI use, so every client on the machine
//!   advertises one node id.
//! - **Which event signals a reload.** `peer-ops-changed` — the same
//!   event the `notify` watcher (`fs_watcher.rs`) emits, so the frontend
//!   reload path is reused verbatim whichever delivery path wins.
//! - **Where the transport lands.** The swap-capable `AppState` slots:
//!   one `dyn SyncTransport` clone (announce / shutdown / peer-health)
//!   and one concrete clone for the pairing commands.
//!
//! ## Best-effort
//!
//! Every failure here (no `$HOME`, unreadable identity, transport build
//! error) is logged and swallowed. Sync degrades to the filesystem
//! watcher; the editor keeps working. iroh is never allowed to block or
//! abort the boot path.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use outl_actions::SyncTransport;
use outl_core::id::ActorId;
use outl_sync_iroh::TransportOutcome;
use outl_tauri_shared::iroh_sync::start_with_reload_bridge;
use parking_lot::Mutex;
use tauri::AppHandle;
use tracing::{info, warn};

/// `true` while this process lost the device endpoint election to another
/// local outl process (typically `outl mcp serve`, which Claude Desktop
/// launches at login and which therefore gets there first).
///
/// It exists because "no iroh transport wired" has two very different causes
/// that the empty `AppState` slots cannot tell apart: the user opted out
/// (`[sync] transport = "file"`), or a co-resident process holds the endpoint.
/// Only the second one deserves an explanation in the UI — see
/// [`endpoint_held_by_another_process`].
///
/// A process global rather than an `AppState` field, and the four questions
/// root `CLAUDE.md` invariant 9 asks of state that leaves its old home:
/// **written** only by [`wire_iroh_transport`] (one writer, one call per
/// wiring pass), **read** by `commands::peers`, **isolated** trivially because
/// it is per-process and no test binds a transport, and **cleaned up** by
/// being *rewritten* on every call — a workspace swap that wins the endpoint
/// clears it instead of leaving a stale warning behind.
static ENDPOINT_BUSY: AtomicBool = AtomicBool::new(false);

/// Whether the missing iroh transport is explained by another local process
/// holding the device endpoint (as opposed to P2P being switched off).
pub(crate) fn endpoint_held_by_another_process() -> bool {
    ENDPOINT_BUSY.load(Ordering::Relaxed)
}

/// Wire the iroh transport into the running app when it is this process's
/// to bind. Called from the boot opener (and from `set_workspace` on a
/// swap) once the workspace root is known — the transport needs the
/// root to write peer ops into `<root>/ops/`.
///
/// On success, stores the transport in `slot` (so the announce /
/// shutdown / peer-health paths can reach it) and in `pairing_slot`
/// (the concrete clone the pairing commands need), and spawns the
/// bridge thread that turns the transport's "peer ops landed" signal
/// into the `peer-ops-changed` event.
///
/// Returns silently (a no-op) when P2P is off, when another outl process on
/// this device already holds the endpoint, or when any step fails — the
/// filesystem watcher already covers detection in all three cases.
///
/// The endpoint-busy case additionally flips [`endpoint_held_by_another_process`]
/// so the peer commands can explain the dark sync dot instead of reporting
/// every paired device as offline, and so the pairing commands know to fall
/// back to a one-shot endpoint (`commands::peers`).
pub(crate) fn wire_iroh_transport(
    slot: &Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    pairing_slot: &Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    workspace_root: PathBuf,
    actor: ActorId,
    app: AppHandle,
) {
    // Rewritten on every pass, not just set — a swap that wins the endpoint has
    // to clear a warning left by the workspace before it.
    ENDPOINT_BUSY.store(false, Ordering::Relaxed);

    let transport = match outl_sync_iroh::build_default_transport(&workspace_root) {
        Ok(TransportOutcome::Ready(t)) => t,
        Ok(TransportOutcome::EndpointBusy) => {
            ENDPOINT_BUSY.store(true, Ordering::Relaxed);
            // `warn!`, not `info!`: this is a degraded mode with two effects the
            // user can see and would otherwise have no explanation for — the
            // sync dot never turns green (no live transport means no
            // `peer_health()`), and Refresh cannot force a P2P pass. Both are
            // reported back through `commands::peers`.
            warn!(
                "another outl process on this device (typically `outl mcp serve`) \
                 holds the iroh endpoint; this window syncs through the shared \
                 ops/ dir instead. P2P peer status will read as offline and \
                 Refresh cannot force a pass until that process exits."
            );
            return;
        }
        Ok(TransportOutcome::Disabled) => return,
        Err(e) => {
            warn!("iroh sync unavailable, using filesystem watcher: {e}");
            return;
        }
    };

    // Bridge the transport's "peer ops landed" signal to the SAME event
    // the `notify` watcher emits so the frontend reload path is reused.
    start_with_reload_bridge(&transport, workspace_root, actor, app, "peer-ops-changed");

    // Keep the concrete clone for pairing (reuses the live endpoint) and the
    // `dyn` clone for announce / shutdown / peer_health. `IrohSyncTransport`
    // is `Clone` (internally `Arc`-backed), so both handles drive the one
    // running transport.
    *pairing_slot.lock() = Some(transport.clone());
    *slot.lock() = Some(Arc::new(transport) as Arc<dyn SyncTransport>);
    info!("iroh sync transport wired");
}
