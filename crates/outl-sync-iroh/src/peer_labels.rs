//! Human-readable labels for peers, kept out of [`peers`] so that store module
//! stays focused on persistence.
//!
//! These are inherent methods on [`PeerEntry`] / [`PeersStore`] and a free
//! helper for the sync INFO lines: a peer is identified to a *human* by its
//! alias when one was set at pairing time, else by a short hex prefix of its
//! node id. The label is purely display state — `peers.json` stays the single
//! source of the alias; nothing here writes.

use std::fmt::Display;
use std::path::Path;

use crate::peers::{workspace_peers_path, PeerEntry, PeersStore};

impl PeerEntry {
    /// User-facing label: the alias when set, else a short hex prefix.
    pub fn display_label(&self) -> String {
        self.alias.clone().unwrap_or_else(|| self.short_hex())
    }

    /// Log-facing label: `"macbook-pro (a1b2c3d4)"` or just `"a1b2c3d4"`.
    ///
    /// Always carries the hex prefix so a line can be grep-correlated back to
    /// `peers.json` or `outl peer list` without losing the human name.
    pub fn log_label(&self) -> String {
        let hex = self.short_hex();
        match &self.alias {
            Some(name) => format!("{name} ({hex})"),
            None => hex,
        }
    }

    fn short_hex(&self) -> String {
        self.node_id[..self.node_id.len().min(8)].to_string()
    }
}

impl PeersStore {
    /// Resolve a node id to its log-facing label (`"alias (hex)"` or bare hex).
    ///
    /// Falls back to the truncated hex prefix when the id is unknown to this
    /// store (an unpaired or revoked peer on the inbound path).
    pub fn log_label_for(&self, node_id: &str) -> String {
        self.list()
            .iter()
            .find(|p| p.node_id == node_id)
            .map(|p| p.log_label())
            .unwrap_or_else(|| node_id[..node_id.len().min(8)].to_string())
    }

    /// Resolve a node id to its user-facing label (alias or short hex).
    pub fn display_label_for(&self, node_id: &str) -> String {
        self.list()
            .iter()
            .find(|p| p.node_id == node_id)
            .map(|p| p.display_label())
            .unwrap_or_else(|| node_id[..node_id.len().min(8)].to_string())
    }
}

/// Log-facing label for the delta-sync INFO lines: resolve `node_id` against
/// the workspace peer store, falling back to `fallback` (the short hex) when
/// the store can't be read. Lazy — `tracing` only evaluates the `info!`
/// arguments once the level passes, so a quiet pass performs no disk read.
///
/// The `SyncProgress.peer` field keeps the short hex separately: the desktop
/// resolves the alias client-side by prefix-matching it (`aliasFor`).
pub(crate) fn sync_log_label<T: Display>(
    workspace_root: &Path,
    node_id: &T,
    fallback: &str,
) -> String {
    PeersStore::load_or_default(&workspace_peers_path(workspace_root))
        .map(|s| s.log_label_for(&node_id.to_string()))
        .unwrap_or_else(|_| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inline `PeerEntry` with a fixed hex id, for the label-format contract.
    fn entry_with(alias: Option<&str>) -> PeerEntry {
        PeerEntry {
            node_id: "a1b2c3d4e5f60718".to_string(),
            alias: alias.map(|s| s.to_string()),
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// The exact strings `status.rs` / `revoke.rs` emit. A revert to bare
    /// `node_id` / `fmt_short()` changes these, so the sites' output is pinned.
    #[test]
    fn log_and_display_label_format() {
        let named = entry_with(Some("macbook-pro"));
        assert_eq!(named.log_label(), "macbook-pro (a1b2c3d4)");
        assert_eq!(named.display_label(), "macbook-pro");

        let anon = entry_with(None);
        assert_eq!(anon.log_label(), "a1b2c3d4");
        assert_eq!(anon.display_label(), "a1b2c3d4");
    }

    /// Store lookups resolve to the entry's label while paired, and fall back to
    /// the 8-char hex prefix for an id this store never knew (unpaired/revoked).
    #[test]
    fn store_label_lookup_resolves_known_and_unknown() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("peers.json");
        let mut store = PeersStore::load_or_default(&path).expect("store");
        store
            .add(entry_with(Some("macbook-pro")))
            .expect("seed entry");

        let known = "a1b2c3d4e5f60718";
        let unknown = "ffffffffffffffff";
        assert_eq!(store.log_label_for(known), "macbook-pro (a1b2c3d4)");
        assert_eq!(store.display_label_for(known), "macbook-pro");
        assert_eq!(store.log_label_for(unknown), "ffffffff");
        assert_eq!(store.display_label_for(unknown), "ffffffff");
    }
}
