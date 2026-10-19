//! Crash-reporting consent commands (RFC 0377). Their own macro because
//! `catalog.rs` sits at its size ceiling; same shape as the rest.
//!
//! **Both** GUI clients register it, and unlike every other module here
//! that is not a convenience. Reporting is on by default, so a client
//! without these two commands is a client with no opt-out at all —
//! `outl_shortcuts::capability_support` says `CrashReportingOptOut` is
//! supported on the desktop and on mobile, and this macro is what makes
//! that true. `tests/command_parity.rs` scans every `src/wrappers/*.rs`,
//! so a client dropping the module fails the build.

/// `get_telemetry_enabled` / `set_telemetry_enabled`: read and write
/// `[telemetry] enabled` in the global config.
///
/// Both entries are `plain` (config only, no workspace), so `$state` is
/// never expanded — kept for call-site uniformity with the other
/// modules.
#[macro_export]
macro_rules! telemetry_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Whether this device sends crash reports (default `true`).
            fn get_telemetry_enabled() -> bool
                => plain $crate::commands::telemetry::get_telemetry_enabled;

            /// Persist the consent flag; returns what is now on disk, so
            /// the UI moves the switch only after the write lands.
            fn set_telemetry_enabled(enabled: bool)
                -> ::std::result::Result<bool, String>
                => plain $crate::commands::telemetry::set_telemetry_enabled;
        }
    };
}
