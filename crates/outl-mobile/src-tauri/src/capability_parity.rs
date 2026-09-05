//! Pins mobile's declared column in
//! [`outl_shortcuts::capability_support`] against commands this crate
//! actually registers, so the direct `outl-shortcuts` dependency
//! added for RFC 0253 is load-bearing rather than a `use` statement
//! waiting for the next `cargo machete` run to drop it.
//!
//! Before this dependency was direct, `outl-shortcuts` only reached
//! mobile transitively (via `outl-plugins` / `outl-tauri-shared`), so
//! mobile's column in the shared parity catalog was a claim nobody's
//! build here could falsify. This module narrows that gap, not
//! closes it: each assertion below fails at compile time if the
//! named command's function item disappears from `crate::commands`,
//! and at runtime if the catalog's `Full` verdict for this client
//! changes without this crate noticing.
//!
//! **What this does not catch:** a command dropped from `lib.rs`'s
//! `invoke_handler!` list while its function stays defined. The
//! function-item reference only proves the item still exists in
//! source — it says nothing about whether Tauri still dispatches to
//! it. Catching that would mean inspecting the `generate_handler!`
//! macro invocation itself, which this module doesn't attempt.
//!
//! Test-only — there is nothing here for the running app to call.

#[cfg(test)]
mod tests {
    use outl_shortcuts::{capability_support, Capability, Support};

    /// Referencing a command as a function *item* (never calling it)
    /// proves the path still resolves in `crate::commands`, without
    /// needing a live `tauri::State`. It does **not** prove the
    /// command is registered in `lib.rs`'s `invoke_handler!` — see
    /// this module's doc comment for that limit.
    #[test]
    fn mobile_capability_column_is_backed_by_real_commands() {
        let _ = crate::commands::plugin_registry_list;
        assert_eq!(
            capability_support(Capability::PluginMarketplace).mobile,
            Support::Full,
            "outl-mobile registers plugin_registry_list (the marketplace \
             browse tab in PluginSheet.tsx) — the shared catalog must not \
             downgrade this client's PluginMarketplace support without this \
             crate's own build noticing",
        );

        let _ = crate::commands::list_templates_cmd;
        let _ = crate::commands::instantiate_template_at;
        assert_eq!(
            capability_support(Capability::Templates).mobile,
            Support::Full,
            "outl-mobile registers list_templates_cmd / instantiate_template_at \
             (TemplateSheet) — the catalog's Templates verdict for mobile must \
             track that.",
        );

        let _ = crate::commands::attach_asset;
        let _ = crate::commands::import_asset_file;
        assert_eq!(
            capability_support(Capability::Assets).mobile,
            Support::Full,
            "outl-mobile registers attach_asset / import_asset_file (Journal.tsx's \
             paste + attach path) — the catalog's Assets verdict for mobile must \
             track that.",
        );

        let _ = crate::commands::outl_peer_pair_host;
        let _ = crate::commands::outl_peer_pair_join;
        assert_eq!(
            capability_support(Capability::PeerPairing).mobile,
            Support::Full,
            "outl-mobile registers outl_peer_pair_host / outl_peer_pair_join \
             (DevicesSheet's scan + \"show my QR\" paths) — the catalog's \
             PeerPairing verdict for mobile must track that.",
        );

        let _ = crate::commands::reminder_action_catalog;
        assert_eq!(
            capability_support(Capability::ReminderNotificationActions).mobile,
            Support::Full,
            "outl-mobile registers reminder_action_catalog (the category + buttons \
             the frontend hands to registerActionTypes, and the same ids \
             deliver_due_reminders stamps onto every banner) — the catalog's \
             ReminderNotificationActions verdict for mobile must track that.",
        );
    }
}
