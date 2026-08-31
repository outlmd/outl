//! Workspace-identity rotation — the only revocation that holds against a
//! device you no longer control.
//!
//! ## Why rotation, and not a propagated tombstone
//!
//! `PeersStore::remove` revokes a device **on the machine that ran it**. That
//! is the right primitive for retiring a laptop you still have, and it is not
//! enough for one you do not: every other paired device keeps its own list, so
//! the removed device keeps syncing with all of them.
//!
//! The obvious next step is to sign the tombstone and gossip it. It was
//! rejected, and the reason matters more than the mechanism: it would let any
//! paired device evict any other. In the scenario this exists for — a stolen
//! laptop — the attacker holds a paired device, so they would get to revoke
//! *your* devices first. That trades "cannot revoke" for "whoever moves first
//! wins", which is not an improvement when the other party is the one paying
//! attention.
//!
//! Rotation inverts it. The new [`WorkspaceId`] never leaves the devices you
//! re-pair, so a device that is not physically in your hands cannot learn it.
//! There is no race to win.
//!
//! ## Why this is a dozen lines
//!
//! Every mechanism it needs already exists and is already tested:
//!
//! - The gossip topic is `blake3(workspace_id)`, so a rotated device stops
//!   sharing a topic with the old one — they never even discover each other.
//! - `SyncProtocolHandler::serve` validates the request's workspace id against
//!   the local one and closes `workspace-mismatch`.
//! - Pairing already makes a joiner adopt the host's id, so re-pairing spreads
//!   the new id with no new code.
//!
//! Rotation is composition, not a new protocol. That is the argument for it:
//! the alternative would have added a wire format, a signature scheme and a
//! trust question, to end up weaker.
//!
//! ## What it does not do
//!
//! The revoked device keeps the copy of the graph it already synced. Rotation
//! stops it receiving anything **new**; nothing can un-send history. Say so
//! plainly wherever this is surfaced — a user who thinks their notes came back
//! is worse off than one who knows they did not.
//!
//! See [RFC 0155](../../docs/rfcs/0155-peer-trust.md) and
//! [issue #158](https://github.com/outlmd/outl/issues/158).

use std::path::Path;

use anyhow::{Context, Result};
use outl_core::WorkspaceId;

use crate::peers::{workspace_peers_path, PeersStore};

/// Rotate this workspace's identity and drop every paired device.
///
/// Returns how many devices were unpaired. The new id is deliberately not
/// returned: the only honest place to read it is `<root>/.outl/workspace-id`,
/// and a caller trusting a returned value over the file would report a
/// rotation that a failed write never made.
///
/// **Order is load-bearing: the id is persisted first.** A crash between the
/// two steps leaves a new id with a stale peer list, and a stale peer list is
/// inert — those devices carry the old id and are refused `workspace-mismatch`
/// on the next connection. The reverse order would leave the old id with no
/// peers, which is a workspace that still answers to every device that was
/// ever paired with it.
///
/// Idempotent in the sense that matters: running it twice rotates twice, and
/// both rotations lock out everything that came before.
pub fn rotate_workspace_identity(workspace_root: &Path) -> Result<usize> {
    WorkspaceId::new()
        .write(workspace_root)
        .context("persist the new workspace id")?;

    let peers_path = workspace_peers_path(workspace_root);
    let mut store = PeersStore::load_or_default(&peers_path).context("load peers.json")?;
    let unpaired = store.list().len();
    // Tombstones are deliberately left in place. They cost nothing, and they
    // are a second line of defence for the window between rotating here and
    // re-pairing a device that a peer might still gossip about.
    for node_id in store
        .list()
        .iter()
        .map(|p| p.node_id.clone())
        .collect::<Vec<_>>()
    {
        store
            .remove(&node_id)
            .with_context(|| format!("unpair {node_id}"))?;
    }

    Ok(unpaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::PeerEntry;

    fn peer(node_id: &str) -> PeerEntry {
        PeerEntry {
            node_id: node_id.to_string(),
            alias: None,
            relay_url: Some("https://relay.example/".to_string()),
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn rotation_changes_the_id_and_unpairs_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let before = WorkspaceId::read_or_create(root).unwrap();

        let mut store = PeersStore::load_or_default(&workspace_peers_path(root)).unwrap();
        store.add(peer("aaa")).unwrap();
        store.add(peer("bbb")).unwrap();

        let unpaired = rotate_workspace_identity(root).unwrap();
        assert_eq!(unpaired, 2);

        let after = WorkspaceId::read_or_create(root).unwrap();
        assert_ne!(after, before, "the id on disk must actually change");

        let store = PeersStore::load_or_default(&workspace_peers_path(root)).unwrap();
        assert!(
            store.list().is_empty(),
            "every device must be unpaired — a peer left behind is a device \
             that still syncs until it happens to be refused",
        );
    }

    #[test]
    fn a_revoked_device_keeps_its_tombstone_after_rotation() {
        // Defence in depth for the window between rotating and re-pairing: a
        // device you removed before rotating must not be re-added by a peer
        // that is still gossiping about it.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = workspace_peers_path(root);

        let mut store = PeersStore::load_or_default(&path).unwrap();
        store.add(peer("lost-laptop")).unwrap();
        store.remove("lost-laptop").unwrap();
        assert!(store.is_revoked("lost-laptop"));

        rotate_workspace_identity(root).unwrap();

        let store = PeersStore::load_or_default(&path).unwrap();
        assert!(
            store.is_revoked("lost-laptop"),
            "rotation must not wipe an existing revocation",
        );
    }

    #[test]
    fn rotating_twice_locks_out_both_previous_identities() {
        // Each rotation must mint a fresh id, not toggle between two.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let start = WorkspaceId::read_or_create(root).unwrap();
        rotate_workspace_identity(root).unwrap();
        let after_first = WorkspaceId::read_or_create(root).unwrap();
        rotate_workspace_identity(root).unwrap();
        let after_second = WorkspaceId::read_or_create(root).unwrap();

        assert_ne!(after_first, start);
        assert_ne!(after_second, after_first, "each rotation mints a fresh id");
        assert_ne!(after_second, start, "not a toggle between two ids");
    }

    #[test]
    fn rotating_an_unpaired_workspace_is_harmless() {
        // Running it "just in case" must not error or invent peers.
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(rotate_workspace_identity(tmp.path()).unwrap(), 0);
    }
}
