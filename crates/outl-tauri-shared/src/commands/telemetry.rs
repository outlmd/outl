//! Crash-reporting consent: read and write `[telemetry] enabled`.
//!
//! RFC 0377 section 7. Reporting is **on by default**, so a client that
//! cannot turn it off is a client whose user has no opt-out — which is
//! the footing the whole feature rests on, not a missing convenience.
//! Both GUI clients therefore register these two commands, and
//! `outl_shortcuts::capability_support`'s
//! `Capability::CrashReportingOptOut` is where that verdict is recorded
//! per client.
//!
//! Pure config I/O: no `AppHost`, no workspace, no lock. The flag lives
//! in `~/.config/outl/config.toml` rather than in the op log because
//! `outl-telemetry` reads it at process start, before a workspace
//! exists — RFC 0377 section 4 argues that departure from root
//! `CLAUDE.md` invariant 7.
//!
//! **This is the only writer**, and that is load-bearing. The desktop's
//! flat `Settings` adapter deliberately does not model the field and
//! restores `[telemetry]` from disk on every modal save
//! (`outl-desktop/src-tauri/src/settings.rs`), the same split
//! `[display] backlinks_order` has from `set_backlinks_order`. Modelling
//! it in both places either makes this toggle inert or flips a user's
//! opt-out back on with every unrelated settings save, since the default
//! is `true`.
//!
//! No unit test here on purpose: both functions go through
//! `outl_config::load` / `save`, which resolve the real
//! `~/.config/outl/config.toml` unless `$XDG_CONFIG_HOME` is set, and
//! this crate has no harness that gives a test its own copy. Writing the
//! developer's actual consent flag from the suite is the
//! invariant-9 failure the device store already paid for. The clients'
//! Vitest suites cover the UI half against a mocked command.

/// Whether this device sends crash reports.
///
/// `true` for a config that never mentioned `[telemetry]` — the default
/// is on. **Not** `load().telemetry.enabled`: that answers from defaults
/// for a file it could not parse, so a switch reading "on" would be the
/// UI agreeing with a fallback rather than with the user. The gate in
/// each binary asks the same function, so the switch and the reporter
/// cannot disagree about an unreadable file.
pub fn get_telemetry_enabled() -> bool {
    outl_config::telemetry_enabled()
}

/// Persist the consent flag and return what is now on disk.
///
/// Returns the saved value rather than `()` so a client can move the
/// switch *after* the write lands — the shape `set_reminder_settings`
/// uses. A toggle that flips optimistically and then fails to save is a
/// switch reading "off" over a reporter that is still on, which is the
/// one thing this surface must never say.
///
/// Reads the file first and writes back only this one field, so it
/// cannot clobber a hand-set timezone or relay URL.
pub fn set_telemetry_enabled(enabled: bool) -> Result<bool, String> {
    let mut cfg = outl_config::load();
    cfg.telemetry.enabled = enabled;
    outl_config::save(&cfg).map_err(|e| e.to_string())?;
    // Re-read instead of echoing the argument: the value the UI settles
    // on should be the file's answer, not the caller's request.
    Ok(get_telemetry_enabled())
}
