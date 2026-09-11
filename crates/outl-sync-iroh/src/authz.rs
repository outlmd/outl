//! The single owner of "is this dialer an approved device for this workspace?"
//!
//! ## Why this is a module and not three checks
//!
//! One endpoint accepts four ALPNs ([`crate::protocol::SYNC_ALPN`],
//! [`crate::protocol::SNAPSHOT_ALPN`], [`crate::protocol::ASSET_ALPN`],
//! [`crate::protocol::PAIRING_ALPN`]). Issue #158 added a fail-closed
//! `peers.json` check so a device removed with `outl peer remove` stops syncing
//! — and added it **inline in `SYNC_ALPN`'s handler only**. The other two
//! content protocols were mounted on the same endpoint, answered the same
//! dialers, and shipped the same workspace, with no check at all:
//!
//! - `SNAPSHOT_ALPN` returns `snap-<actor>.bin`, the **materialized workspace**
//!   — every page, to any dialer that speaks the ALPN.
//! - `ASSET_ALPN` returns the `assets/` manifest and then every blob in it.
//!
//! So a revoked device kept full read access to the graph and to every uploaded
//! file; only its *writes* were refused. The authorization existed and two of
//! three protocols skipped it, which is what a second copy of a policy always
//! turns into — see [invariant 12](../../../CLAUDE.md): a capability with no
//! recorded per-surface verdict is a gap nobody wrote down.
//!
//! This module is the one verdict. Every protocol that serves workspace content
//! calls [`authorize_dialer`] before it reads a byte off disk.
//!
//! ## Why `peers.json` membership also answers "the right workspace"
//!
//! `SYNC_ALPN` additionally rejects a mismatched `workspace_id` carried in the
//! request body. Snapshot and asset have no request body to carry one, and they
//! do not need it: the peer list this reads is
//! `<workspace_root>/.outl/peers.json`, which is **per workspace**. A device
//! paired into a different workspace is not in *this* workspace's list, so it is
//! refused by node id alone. The `workspace_id` check on `SYNC_ALPN` stays what
//! it has always been — an earlier, cheaper rejection of an obviously wrong
//! peer, not the authorization itself.
//!
//! ## Fail closed, and re-read every time
//!
//! An unreadable `peers.json` is a refusal, never a fallback to open access: we
//! cannot prove the dialer is approved, so we do not serve it. And the file is
//! loaded fresh on **every** inbound connection rather than cached at boot, so
//! `outl peer remove` takes effect on the next connection instead of the next
//! restart.

use std::path::Path;

use iroh::endpoint::Connection;
use tracing::warn;

use crate::protocol::CLOSE_UNKNOWN_PEER;

/// Why a dialer was refused. Both variants deny; they differ only in what to
/// tell the operator, and conflating them is how "the peer list is corrupt"
/// gets read as "that device was revoked".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The dialer's node id is not in this workspace's `peers.json` — never
    /// paired, or removed with `outl peer remove`.
    NotAPeer,
    /// `peers.json` could not be read at all. Denied, because an unprovable
    /// approval is not an approval.
    PeerListUnreadable,
}

impl Refusal {
    /// The wire close reason. Deliberately the SAME bytes for both variants:
    /// the dialer is told it is not paired, and learns nothing about whether
    /// the responder's peer list is healthy.
    pub(crate) fn close_reason(self) -> &'static [u8] {
        b"unknown-peer"
    }
}

/// The verdict itself, blocking, against an explicit `peers.json` path.
///
/// **The only scan of the peer list in the crate.** Callers that already hold
/// the path (the gossip supervisor) call this directly; callers holding a
/// workspace root call [`authorize_dialer`], which is this plus a
/// `spawn_blocking`. A second copy of the scan is how the two answers drift —
/// the first version of this module had one, and it returned a bare `bool`, so
/// the caller that used it could not tell a revoked device from a peer list it
/// had failed to read.
pub(crate) fn authorize_blocking(
    peers_path: &Path,
    node_id: iroh::EndpointId,
) -> Result<(), Refusal> {
    let wanted = node_id.to_string();
    match crate::peers::PeersStore::load_or_default(peers_path) {
        Ok(store) if store.list().iter().any(|p| p.node_id == wanted) => Ok(()),
        Ok(_) => {
            warn!(
                peer = %node_id.fmt_short(),
                "refusing an unknown / revoked peer (not in peers.json)"
            );
            Err(Refusal::NotAPeer)
        }
        // Fail CLOSED: an unreadable peer list cannot prove an approval, and
        // "prove it or refuse" is the only reading of that which stays safe
        // when the file is damaged.
        Err(e) => {
            warn!(
                peer = %node_id.fmt_short(),
                "refusing peer: peers.json unreadable ({e:#})"
            );
            Err(Refusal::PeerListUnreadable)
        }
    }
}

/// Decide whether `remote_id` is an approved device for the workspace at
/// `workspace_root`.
///
/// The blocking `peers.json` read runs on a blocking thread — it takes the
/// peer-file lock, so a contended read must never sit on a tokio worker while
/// another process holds it.
pub(crate) async fn authorize_dialer(
    workspace_root: &Path,
    remote_id: iroh::EndpointId,
) -> Result<(), Refusal> {
    let peers_path = crate::peers::workspace_peers_path(workspace_root);
    match tokio::task::spawn_blocking(move || authorize_blocking(&peers_path, remote_id)).await {
        Ok(verdict) => verdict,
        Err(e) => {
            warn!(
                peer = %remote_id.fmt_short(),
                "refusing peer: peers.json load task failed ({e})"
            );
            Err(Refusal::PeerListUnreadable)
        }
    }
}

/// [`authorize_dialer`] plus the close a refused dialer gets, for the handlers
/// that have nothing to say beyond "go away".
///
/// Returns `true` when the dialer may be served. On `false` the connection is
/// already closed with [`CLOSE_UNKNOWN_PEER`] and the caller must return
/// without touching the workspace.
pub(crate) async fn authorize_or_close(conn: &Connection, workspace_root: &Path) -> bool {
    match authorize_dialer(workspace_root, conn.remote_id()).await {
        Ok(()) => true,
        Err(refusal) => {
            conn.close(CLOSE_UNKNOWN_PEER.into(), refusal.close_reason());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::{PeerEntry, PeersStore};

    fn entry(node_id: &str) -> PeerEntry {
        PeerEntry {
            node_id: node_id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn node_id() -> iroh::EndpointId {
        iroh::SecretKey::generate().public()
    }

    #[tokio::test]
    async fn a_paired_peer_is_authorized() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let peer = node_id();
        let path = crate::peers::workspace_peers_path(tmp.path());
        let mut store = PeersStore::load_or_default(&path).expect("load");
        store.add(entry(&peer.to_string())).expect("add");

        assert!(authorize_dialer(tmp.path(), peer).await.is_ok());
    }

    #[tokio::test]
    async fn an_unknown_dialer_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            authorize_dialer(tmp.path(), node_id()).await,
            Err(Refusal::NotAPeer),
        );
    }

    #[tokio::test]
    async fn a_revoked_peer_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let peer = node_id();
        let path = crate::peers::workspace_peers_path(tmp.path());
        let mut store = PeersStore::load_or_default(&path).expect("load");
        store.add(entry(&peer.to_string())).expect("add");
        assert!(store.remove(&peer.to_string()).expect("remove"));

        assert_eq!(
            authorize_dialer(tmp.path(), peer).await,
            Err(Refusal::NotAPeer),
        );
    }

    #[tokio::test]
    async fn an_unreadable_peer_list_denies_rather_than_opens() {
        // Fail CLOSED. A parse error here used to be the difference between
        // "cannot prove approval" and "serve everyone", and only one of those
        // is safe to pick by accident.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = crate::peers::workspace_peers_path(tmp.path());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"{ this is not json").expect("write junk");

        assert_eq!(
            authorize_dialer(tmp.path(), node_id()).await,
            Err(Refusal::PeerListUnreadable),
        );
    }

    /// The blocking verdict and the async one agree, case for case.
    ///
    /// They used to be two scans of the same file: `authorize_dialer` had its
    /// own closure and the blocking form returned a bare `bool`, which
    /// collapsed `PeerListUnreadable` into `NotAPeer` for its one caller (the
    /// gossip supervisor). Both denied, so nothing leaked — what was lost was
    /// the operator's ability to tell an intact revocation from a damaged
    /// credential store, which is the only distinction this file carries.
    #[tokio::test]
    async fn the_blocking_verdict_matches_the_async_one_case_for_case() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = crate::peers::workspace_peers_path(tmp.path());

        // ALLOW.
        let peer = node_id();
        let mut store = PeersStore::load_or_default(&path).expect("load");
        store.add(entry(&peer.to_string())).expect("add");
        assert_eq!(authorize_blocking(&path, peer), Ok(()));
        assert_eq!(authorize_dialer(tmp.path(), peer).await, Ok(()));

        // DENY — a device that is not listed.
        let stranger = node_id();
        assert_eq!(authorize_blocking(&path, stranger), Err(Refusal::NotAPeer));
        assert_eq!(
            authorize_dialer(tmp.path(), stranger).await,
            Err(Refusal::NotAPeer)
        );

        // DENY — and a damaged list must NOT read as "that device was revoked".
        std::fs::write(&path, b"{ this is not json").expect("write junk");
        assert_eq!(
            authorize_blocking(&path, peer),
            Err(Refusal::PeerListUnreadable)
        );
        assert_eq!(
            authorize_dialer(tmp.path(), peer).await,
            Err(Refusal::PeerListUnreadable)
        );
    }

    #[test]
    fn both_refusals_look_identical_on_the_wire() {
        // A dialer must not be able to tell a revocation from a broken peer
        // file — that difference is the responder's business.
        assert_eq!(
            Refusal::NotAPeer.close_reason(),
            Refusal::PeerListUnreadable.close_reason(),
        );
    }
}
