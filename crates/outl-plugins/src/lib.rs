//! # outl-plugins
//!
//! The plugin system shared by every outl client (TUI, desktop, mobile, CLI).
//!
//! A plugin is bundled JavaScript described by a [`manifest::PluginManifest`]
//! (`plugin.json`). It declares the [`capability::Capability`] set it registers
//! and the [`permission::Permission`] set it needs; the user approves
//! permissions on install and the loader intersects capabilities with what the
//! current client implements. Approved state is frozen in the
//! [`lockfile::InstalledPlugins`] (`installed.json`), whose bundle hash is
//! revalidated on every load so a plugin never changes silently.
//!
//! Plugin mutations never touch `outl-core` directly: the host API routes every
//! change through `outl-actions` → `Workspace::apply`, stamped with
//! `actor = "plugin:<id>@<device>"`, so the op log doubles as an audit trail.
//!
//! ## What this crate owns today (scaffold)
//!
//! - Manifest parse + validation ([`manifest`]).
//! - Capability intersection ([`capability`]).
//! - Permission parse + gating ([`permission`]).
//! - The lockfile ([`lockfile`]).
//! - The engine seam ([`runtime::PluginEngine`]) — a Boa-backed engine, the
//!   `PluginHost`, and the op-hook / command dispatch surface are layered on
//!   top.

// `unsafe` is confined to `engine.rs` (Boa capturing natives) with a local
// `#![allow(unsafe_code)]` and a SAFETY note — the rest of the crate is safe.
#![warn(missing_docs)]

pub mod capability;
#[cfg(feature = "js")]
pub mod engine;
pub mod error;
pub mod host;
pub mod loader;
pub mod lockfile;
pub mod manifest;
pub mod model;
#[cfg(feature = "js")]
mod net;
pub mod permission;
pub mod registry;
pub mod runtime;
pub mod secrets;
pub mod settings;

pub use capability::{Capability, CapabilityMatch, ClientCapabilities};
pub use error::{PluginError, Result};
pub use host::{
    CommandEntry, PluginBinding, PluginHost, PluginRun, ToolbarButtonEntry, TransformerEntry,
};
pub use loader::{
    install_from_dir, load_installed, lockfile_path, plugins_dir, uninstall, LoadReport,
};
pub use lockfile::{bundle_hash, InstalledEntry, InstalledPlugins};
pub use manifest::PluginManifest;
pub use model::{HostIntent, LogOpView, MoveTarget, ReadModel, TransformResult};
pub use permission::{NetworkDomain, Permission, PermissionSet};
#[cfg(feature = "registry")]
pub use registry::{marketplace_install, marketplace_list};
pub use registry::{
    set_enabled, MarketplaceItem, RegistryEntry, RegistryError, RegistryIndex,
    DEFAULT_REGISTRY_BASE, DEFAULT_REGISTRY_URL,
};
pub use runtime::{EngineError, PluginEngine};
pub use secrets::{plugin_service, KeyringStore, MemorySecretStore, SecretStore};
pub use settings::{describe as describe_settings, FieldKind, SettingsField};

#[cfg(feature = "js")]
pub use engine::BoaEngine;

/// The plugin-API version this host implements. A plugin whose manifest `api`
/// range does not match this is refused at load (`outl plugin install` and the
/// in-client loader both check it). Bumped only on a breaking API change, never
/// in a patch.
pub const HOST_API_VERSION: semver::Version = semver::Version::new(1, 0, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_api_is_one_point_oh() {
        assert_eq!(HOST_API_VERSION, semver::Version::new(1, 0, 0));
    }

    #[test]
    fn end_to_end_manifest_capability_permission() {
        // A manifest the example plugin would ship, run through the three
        // load-bearing checks the loader performs.
        let json = br#"{
            "id": "app.outl.examples.todo-archiver",
            "name": "Todo Archiver",
            "version": "1.0.0",
            "api": "^1.0",
            "main": "index.js",
            "capabilities": ["op-hook", "content-transformer:rich"],
            "permissions": ["read-page", "submit-op"]
        }"#;
        let manifest = PluginManifest::parse(json).unwrap();

        // 1. API compat.
        assert!(manifest.is_api_compatible(&HOST_API_VERSION));

        // 2. Capability intersection against a TUI-like client (no rich render).
        let client: ClientCapabilities = [Capability::OpHook, Capability::SlashCommand]
            .into_iter()
            .collect();
        let m = capability::intersect(&manifest.capabilities, &client);
        assert!(m.granted.contains(&Capability::OpHook));
        assert!(m.missing.contains(&Capability::ContentTransformerRich));

        // 3. Permission gating from the approved set.
        let approved = PermissionSet::new(manifest.permissions.clone());
        assert!(approved.check(&Permission::ReadPage));
        assert!(!approved.check(&Permission::WritePage));
    }
}
