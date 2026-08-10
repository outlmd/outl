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
use std::sync::Arc;

use outl_actions::SyncTransport;
use outl_core::id::ActorId;
use outl_sync_iroh::{LeaseDenied, TransportOutcome};
use outl_tauri_shared::iroh_sync::start_with_reload_bridge;
use parking_lot::Mutex;
use tauri::AppHandle;
use tracing::{info, warn};

/// Why this window has no iroh transport wired, when it has none.
///
/// The empty `AppState` slots cannot tell these apart, and they want opposite
/// answers from `commands::peers`: only [`Self::P2pDisabled`] is the user's own
/// choice, and it is the only one where refusing to pair is right. Collapsing
/// the rest into it told a user whose transport failed to build to go switch on
/// a setting that was already on.
#[derive(Clone, Debug)]
pub(crate) enum NoEndpoint {
    /// `[sync] transport = "file"`: the user opted out of P2P.
    P2pDisabled,
    /// Another local outl process won the endpoint election, typically
    /// `outl mcp serve`, which Claude Desktop launches at login and which
    /// therefore gets there first.
    HeldByAnotherProcess,
    /// We could not get an endpoint for any other reason: the lease file could
    /// not be opened (permission, read-only mount), or building the transport
    /// failed (unreadable identity or peer store). Carries the rendered reason.
    /// Nobody holds the endpoint in this state, so no process exiting fixes it.
    Unavailable(String),
}

/// The current reason, or `None` while this process holds the endpoint (or has
/// not tried yet, before the first workspace opens).
///
/// A process global rather than an `AppState` field, and the four questions
/// root `CLAUDE.md` invariant 9 asks of state that leaves its old home:
/// **written** only by [`wire_iroh_transport`] (one writer, one call per
/// wiring pass), **read** by `commands::peers`, **isolated** trivially because
/// it is per-process and no test binds a transport, and **cleaned up** by
/// being *rewritten* on every call, so a workspace swap that wins the endpoint
/// clears it instead of leaving a stale warning behind.
static NO_ENDPOINT: std::sync::Mutex<Option<NoEndpoint>> = std::sync::Mutex::new(None);

/// Record (or clear, with `None`) why this window has no endpoint.
///
/// A poisoned lock is recovered rather than propagated: this is one advisory
/// string for the UI, and losing it would turn a degraded sync into a panic.
fn set_no_endpoint(reason: Option<NoEndpoint>) {
    match NO_ENDPOINT.lock() {
        Ok(mut slot) => *slot = reason,
        Err(poisoned) => *poisoned.into_inner() = reason,
    }
}

/// Why the iroh transport is missing, for the commands that have to explain it.
/// `None` means this process holds the endpoint (nothing to explain).
pub(crate) fn no_endpoint_reason() -> Option<NoEndpoint> {
    match NO_ENDPOINT.lock() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
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
/// Every no-transport path additionally records *why* in
/// [`no_endpoint_reason`], so the peer commands can explain the dark sync dot
/// instead of reporting every paired device as offline, and so the pairing
/// commands can tell the user's own opt-out (refuse) from a lost election or a
/// broken build (fall back to a one-shot endpoint) in `commands::peers`.
pub(crate) fn wire_iroh_transport(
    slot: &Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    pairing_slot: &Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    workspace_root: PathBuf,
    actor: ActorId,
    app: AppHandle,
) {
    // Rewritten on every pass, not just set: a swap that wins the endpoint has
    // to clear a warning left by the workspace before it.
    set_no_endpoint(None);

    let transport = match outl_sync_iroh::build_default_transport(&workspace_root) {
        Ok(TransportOutcome::Ready(t)) => t,
        Ok(TransportOutcome::EndpointBusy(LeaseDenied::HeldByAnotherProcess)) => {
            set_no_endpoint(Some(NoEndpoint::HeldByAnotherProcess));
            // `warn!`, not `info!`: this is a degraded mode with two effects the
            // user can see and would otherwise have no explanation for. The
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
        Ok(TransportOutcome::EndpointBusy(denied)) => {
            // No holder to wait for: the lease could not be arbitrated at all,
            // so this window stays off the wire until the device directory is
            // fixed. Same degradation, different sentence for the user.
            warn!(
                "this window has no iroh endpoint: {denied}; syncing through the shared ops/ dir"
            );
            set_no_endpoint(Some(NoEndpoint::Unavailable(denied.to_string())));
            return;
        }
        Ok(TransportOutcome::Disabled) => {
            set_no_endpoint(Some(NoEndpoint::P2pDisabled));
            return;
        }
        Err(e) => {
            warn!("iroh sync unavailable, using filesystem watcher: {e}");
            set_no_endpoint(Some(NoEndpoint::Unavailable(e.to_string())));
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
