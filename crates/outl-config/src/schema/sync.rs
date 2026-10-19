//! Which transport moves the per-actor op log between devices.

use serde::{Deserialize, Serialize};

/// Which sync transport a client wires up at boot.
///
/// `lowercase` serde so the TOML reads `transport = "file"` /
/// `transport = "iroh"` — matching how a user thinks of them, not
/// the Rust variant casing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncTransportKind {
    /// File-based transport (iCloud Drive / shared filesystem). The opt-out
    /// from iroh: set `transport = "file"`. Still fully supported.
    File,
    /// iroh P2P transport (QUIC + hole punching). The default — P2P is
    /// outl's primary sync. Override with `transport = "file"`.
    #[default]
    Iroh,
}

/// Sync section. Controls which transport moves the per-actor op log
/// between devices. Missing `[sync]` falls back to [`SyncTransportKind::Iroh`]
/// (P2P is outl's primary sync); `transport = "file"` is the explicit opt-out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Transport to use. Defaults to [`SyncTransportKind::Iroh`].
    pub transport: SyncTransportKind,

    /// Optional relay URL for the `iroh` transport. `None` (or an
    /// empty string in the TOML, normalized to `None` on read) means
    /// use outl's default relay (`use1-1.relay.avelino.outl.iroh.link`). Ignored by the
    /// `file` transport.
    pub relay_url: Option<String>,
}

impl SyncConfig {
    /// The configured relay URL, with empty strings treated as
    /// "unset". A user who writes `relay_url = ""` in TOML to mean
    /// "use the defaults" gets `None`, same as omitting the key.
    pub fn relay_url(&self) -> Option<&str> {
        self.relay_url.as_deref().filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn empty_config_defaults_to_iroh_transport() {
        // P2P is outl's primary sync, so a missing [sync] section defaults to
        // iroh. `transport = "file"` is the explicit opt-out.
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_section_parses_file_transport() {
        let c: Config = toml::from_str("[sync]\ntransport = \"file\"\n").unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::File);
    }

    #[test]
    fn sync_section_parses_iroh_transport() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
"#,
        )
        .unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        // No relay set → falls back to defaults (None).
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_empty_relay_url_normalizes_to_none() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
relay_url = ""
"#,
        )
        .unwrap();
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_relay_url_is_returned_when_set() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
relay_url = "https://relay.example"
"#,
        )
        .unwrap();
        assert_eq!(c.sync.relay_url(), Some("https://relay.example"));
    }
}
