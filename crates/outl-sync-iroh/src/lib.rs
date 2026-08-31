//! iroh-based P2P sync transport for outl.
//!
//! The main entry point is [`IrohSyncTransport`], which implements
//! [`outl_actions::SyncTransport`] using iroh QUIC + iroh-gossip.
//!
//! ## Quick start
//!
//! [`build_default_transport`] is the whole recipe: it reads the `[sync]`
//! config, takes the device endpoint lease, and loads the device identity
//! (`~/.outl/identity.key`) plus the per-workspace peer store. Never
//! assemble those pieces by hand — a device may bind **one** iroh endpoint, and
//! a second one on the same node id breaks the holder's sync in both
//! directions (see [`EndpointLease`]).
//!
//! ```ignore
//! use outl_sync_iroh::{build_default_transport, TransportOutcome};
//! use outl_actions::SyncEngine;
//! use std::sync::mpsc;
//!
//! match build_default_transport(&workspace_root)? {
//!     TransportOutcome::Ready(transport) => {
//!         let engine =
//!             SyncEngine::with_transport(workspace_root, actor, Box::new(transport));
//!         let (tx, rx) = mpsc::channel();
//!         engine.start_transport(tx);
//!         // Now rx fires whenever peer ops arrive and the workspace is ready to reload.
//!     }
//!     // No endpoint for this process: another one got here first, or the
//!     // lease file could not be opened at all (`why` says which). Not a
//!     // failure: run `outl_actions::FileSyncTransport` instead and converge
//!     // through the shared `ops/` dir. Tell the user which of the two it is.
//!     TransportOutcome::EndpointBusy(why) => { /* fall back to the file transport */ }
//!     // `[sync] transport = "file"` — the user opted out of P2P.
//!     TransportOutcome::Disabled => {}
//! }
//! ```
//!
//! A caller with its own identity (mobile, whose key lives in the app sandbox
//! rather than `~/.outl`) passes that path to [`build_transport`] instead;
//! everything else is identical.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bind;
mod coordination;
mod device;
mod engine;
mod engine_assets;
mod engine_catchup;
mod engine_gossip;
mod engine_membership;
mod engine_pairing;
mod engine_snapshot;
mod engine_sync;
mod health;
mod identity;
mod lease;
mod oplog;
pub(crate) mod pairing;
pub mod peer_conn;
mod peers;
mod peers_lock;
mod progress;
mod protocol;
mod revoke;
mod status;

#[doc(hidden)]
pub mod test_support;

pub use device::{build_default_transport, build_transport, default_device_dir, TransportOutcome};
pub use engine::IrohSyncTransport;
pub use identity::IrohIdentity;
pub use lease::{EndpointLease, LeaseDenied};
pub use pairing::{
    // `decode_ticket` / `mint_ticket` are public so a test can forge a ticket
    // for a known address without the issued secret — which is exactly the
    // attacker in issue #159, and the only way to exercise the host's re-arm
    // path end to end.
    decode_ticket,
    host_pairing,
    join_pairing,
    mint_ticket,
    PairingSecret,
    WorkspaceAdoption,
};
pub use peers::{migrate_global_peers_if_absent, workspace_peers_path, PeerEntry, PeersStore};
pub use protocol::{ASSET_ALPN, PAIRING_ALPN, SNAPSHOT_ALPN, SYNC_ALPN};
pub use revoke::rotate_workspace_identity;
pub use status::{probe_peers, probe_peers_blocking, PeerProbe, PeerStatus};
