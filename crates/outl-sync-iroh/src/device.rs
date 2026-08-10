//! One place that answers "should this process bind an iroh endpoint, and if
//! so, with what?".
//!
//! Standing a transport up is always the same four steps — load the device
//! identity, run the one-time global→workspace peers migration, load the
//! workspace peer store, read `[sync] relay_url` — and it used to be written
//! out three times (TUI, the shared Tauri backend, `outl sync`). Three copies
//! of a recipe is three chances to drift, and the fourth caller (the MCP
//! server) skipped the recipe entirely, which is issue #220.
//!
//! [`build_transport`] is that recipe, plus the two gates every caller has to
//! respect: the configured transport kind, and the device-wide endpoint lease
//! (see [`EndpointLease`]).

use std::path::{Path, PathBuf};

use anyhow::Context;
use outl_config::SyncTransportKind;

use crate::engine::IrohSyncTransport;
use crate::identity::IrohIdentity;
use crate::lease::{EndpointLease, LeaseDenied};
use crate::peers::{migrate_global_peers_if_absent, workspace_peers_path, PeersStore};

/// What [`build_transport`] decided this process may do.
#[derive(Debug)]
pub enum TransportOutcome {
    /// Bind it. The transport carries the endpoint lease until `start()` hands
    /// it to the endpoint thread, which holds it for exactly as long as the
    /// endpoint runs. A transport built and never started keeps it.
    Ready(IrohSyncTransport),
    /// This process may not bind the device endpoint.
    ///
    /// Do **not** bind one: a second endpoint on the same node id steals the
    /// relay route and breaks the holder's sync in both directions. Fall back
    /// to [`outl_actions::FileSyncTransport`] — ops still converge, through the
    /// shared `ops/` dir and the holder's catch-up pass.
    ///
    /// The [`LeaseDenied`] payload says *why*, because the two reasons read
    /// very differently to a user: the ordinary one is that another local
    /// process won the election, but a lease file that cannot be opened denies
    /// everybody (fail-closed, see [`EndpointLease::try_acquire`]) and names no
    /// holder to wait for. A caller that only degrades can ignore it.
    EndpointBusy(LeaseDenied),
    /// `[sync] transport = "file"` — the user opted out of P2P.
    Disabled,
}

/// `~/.outl` — the device-state directory shared by the CLI, the TUI and the
/// desktop app (identity key, endpoint lease, legacy peer list).
///
/// Mobile is deliberately not a caller: iOS has no meaningful home directory,
/// so it passes its app-sandbox path to [`build_transport`] directly.
///
/// `$OUTL_DEVICE_DIR` redirects it, under an `iroh/` subdir so the device
/// store's own layout stays untouched. This is what gives a test its own copy:
/// the identity key **is** the node id and the lease **is** device-wide, so a
/// suite that used the real `~/.outl` would arbitrate against the developer's
/// running desktop app and mint keys in their home directory. The repo's
/// `.cargo/config.toml` points that variable at `target/`, which is the same
/// mechanism `outl_core::device_dir` uses and for the same reason
/// (root `CLAUDE.md` invariant 9).
///
/// **Setting `$OUTL_DEVICE_DIR` moves the iroh identity too, and that rotates
/// this device's node id.** The variable used to scope only the device store
/// (actor binding, machine id) and never touched `~/.outl/identity.key`, so an
/// existing deployment that exports it — a container, a sandboxed CI job —
/// comes back up under a **new** node id the first time it runs a build with
/// this function in it. Every peer's `peers.json` still lists the old id, so
/// the device reads as permanently offline until it is re-paired.
///
/// That is deliberate, not an oversight: the identity key and the endpoint
/// lease are device-local state about a device-local resource, exactly like the
/// actor binding, so a variable that says "this process is a different device"
/// has to move all three or it is not isolating anything. Point the variable at
/// a persistent path (not a fresh tmpdir per run) if you want a stable node id
/// under it, and re-pair once after the move.
pub fn default_device_dir() -> anyhow::Result<PathBuf> {
    if let Ok(custom) = std::env::var("OUTL_DEVICE_DIR") {
        if !custom.is_empty() {
            return Ok(PathBuf::from(custom).join("iroh"));
        }
    }
    let home = dirs::home_dir().context("$HOME is not set; cannot locate ~/.outl")?;
    Ok(home.join(".outl"))
}

/// [`build_transport`] against this device's usual identity,
/// `~/.outl/identity.key` — one keypair per machine, which is exactly why the
/// clients sharing it must take turns binding an endpoint.
///
/// Every client but mobile calls this; mobile's identity lives in its app
/// sandbox, so it calls [`build_transport`] with that path.
pub fn build_default_transport(workspace_root: &Path) -> anyhow::Result<TransportOutcome> {
    build_transport(&default_device_dir()?.join("identity.key"), workspace_root)
}

/// Decide whether this process may bind an iroh endpoint, and build one if so.
///
/// `identity_path` selects both the node id and (through its parent dir) the
/// lease that arbitrates it — see [`EndpointLease`]. `workspace_root` selects
/// the per-graph peer store (`<root>/.outl/peers.json`); the relay comes from
/// `[sync] relay_url` in the user config.
///
/// Errors are reserved for "the inputs are unusable" (unreadable identity or
/// peer store). Every *decision* is a [`TransportOutcome`], because "someone
/// else owns the endpoint" and "the user turned P2P off" are normal states a
/// client degrades through, not failures to report.
pub fn build_transport(
    identity_path: &Path,
    workspace_root: &Path,
) -> anyhow::Result<TransportOutcome> {
    let cfg = outl_config::load();
    if cfg.sync.transport != SyncTransportKind::Iroh {
        return Ok(TransportOutcome::Disabled);
    }
    // Claim the endpoint BEFORE touching the identity file, so a losing process
    // does no work it is going to throw away.
    let lease = match EndpointLease::try_acquire(identity_path) {
        Ok(lease) => lease,
        Err(denied) => return Ok(TransportOutcome::EndpointBusy(denied)),
    };
    let identity = IrohIdentity::load_or_generate(identity_path)?;
    // The peer list is per-GRAPH; the identity is per-DEVICE. The migration
    // copies any legacy global `~/.outl/peers.json` into the workspace once.
    migrate_global_peers_if_absent(workspace_root);
    let peers = PeersStore::load_or_default(&workspace_peers_path(workspace_root))?;
    let relay_url = cfg.sync.relay_url().map(str::to_string);
    let transport = IrohSyncTransport::new(identity, peers, relay_url).with_endpoint_lease(lease);
    Ok(TransportOutcome::Ready(transport))
}
