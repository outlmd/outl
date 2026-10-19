//! The crash-reporting opt-out has to outlive the process that set it.
//!
//! RFC 0377 promises this test by name. Reporting is **on by default**,
//! so "off" is the only state the user had to ask for — and a toggle
//! that forgets it is not a weaker opt-out, it is no opt-out: the next
//! launch reads the default and starts reporting again, with a switch
//! the user remembers flipping.
//!
//! ## Why the test lives here and not next to the writer
//!
//! `outl_tauri_shared::commands::telemetry` is the GUI writer, and its
//! doc comment says plainly why it has no test of its own:
//! `outl_config::load` / `save` resolve the real
//! `~/.config/outl/config.toml` unless `$XDG_CONFIG_HOME` is set, and
//! that crate has no harness that gives a test its own copy. A test
//! there would write the developer's actual consent flag — the
//! invariant-9 failure the device store already paid for once.
//!
//! This crate owns `config_dir()`, which honours `$XDG_CONFIG_HOME`
//! ahead of every other branch, so here the isolation is one env var.
//! `outl-cli`'s `cli_telemetry.rs` isolates the same way, a process at a
//! time.
//!
//! ## One test per binary, deliberately
//!
//! `set_var` is process-wide and Cargo runs a file's tests on parallel
//! threads, so a second test in this file would race this one's config
//! home. If this file ever needs another case, point it at an explicit
//! path with `save_to` / `load_result_from` instead of adding a second
//! `set_var`.

use outl_config::{Config, ConfigSource};
use tempfile::TempDir;

/// The GUI toggle's exact sequence, then a cold read.
///
/// The write goes through the no-argument `save()` rather than
/// `save_to(path)` on purpose: the path resolution is half of what can
/// break. A `save_to` test would still pass if `config_path()` started
/// pointing somewhere no client reads.
#[test]
fn telemetry_off_survives_a_restart() {
    let home = TempDir::new().expect("tempdir");
    std::env::set_var("XDG_CONFIG_HOME", home.path());

    // Guard the harness, not the feature: if this ever fails, the test
    // below was about to write the developer's own consent flag.
    let path = outl_config::config_path();
    assert!(
        path.starts_with(home.path()),
        "config_path() escaped the test's config home — refusing to write \
         the real one: {}",
        path.display()
    );

    // Whatever reporting defaults to, "off" has to be a departure from
    // it or this test proves nothing.
    assert!(
        Config::default().telemetry.enabled,
        "crash reporting is expected ON by default (RFC 0377); if that \
         changed, this test is now asserting the default survives, which \
         is a different and much weaker claim"
    );

    // `set_telemetry_enabled`'s body: load, touch one field, save.
    let mut cfg = outl_config::load();
    cfg.telemetry.enabled = false;
    outl_config::save(&cfg).expect("saving the opt-out");

    // The restart. A fresh read of the file, through the same resolver
    // every client boots with.
    let reloaded = outl_config::load_result();
    assert_eq!(
        reloaded.source,
        ConfigSource::Parsed,
        "the saved config must read back as the user's own values, not as \
         defaults standing in for an unreadable file: {:?}",
        reloaded.error()
    );
    assert!(
        !reloaded.config.telemetry.enabled,
        "the opt-out did not survive the process that wrote it — the next \
         launch would start reporting again"
    );
}

/// A file we could not read must not be mistaken for a user saying yes.
///
/// This is the second test in a file whose module doc says one, and it is
/// allowed for the reason given there: it points `load_result_from` at its
/// own path and never touches `$XDG_CONFIG_HOME`, so it cannot race the
/// test above.
///
/// What it pins is a trap rather than a feature. `Config::default()` carries
/// `enabled = true`, and `load()` hands back exactly that for a file it could
/// not parse — so reading `.telemetry.enabled` off an unreadable load turns a
/// recorded "off" into consent. The middle assertion asserts the trap is
/// still there, so that if the fallback ever stops defaulting ON this test
/// starts failing instead of passing for a reason that no longer holds.
#[test]
fn an_unreadable_config_does_not_consent() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("config.toml");

    std::fs::write(&path, "[telemetry]\nenabled = false\n").expect("write the opt-out");
    let loaded = outl_config::load_result_from(&path);
    assert!(matches!(loaded.source, ConfigSource::Parsed));
    assert!(
        !loaded.telemetry_enabled(),
        "a parsed `enabled = false` is the user's opt-out"
    );

    // The same file, damaged the way a hand edit or a half-written sync
    // damages it: the section header never closes.
    std::fs::write(&path, "[telemetry\nenabled = false\n").expect("write the damaged file");
    let loaded = outl_config::load_result_from(&path);
    assert!(
        matches!(loaded.source, ConfigSource::Unreadable(_)),
        "an unclosed section header is unreadable, not parsed"
    );
    assert!(
        loaded.config.telemetry.enabled,
        "the fallback config is ON — that is the trap this test exists for, \
         and if it ever stops being true this assertion should be revisited \
         rather than deleted"
    );
    assert!(
        !loaded.telemetry_enabled(),
        "an unreadable config must fail closed: the user's recorded answer is \
         unknown, and the fallback is the opposite of what they may have asked for"
    );

    // A first launch is the one state where the default is the answer.
    let missing = outl_config::load_result_from(&dir.path().join("nothing-here.toml"));
    assert!(matches!(missing.source, ConfigSource::Missing));
    assert!(
        missing.telemetry_enabled(),
        "on by default is the decision RFC 0377 records; only an unreadable \
         file is ambiguous"
    );
}
