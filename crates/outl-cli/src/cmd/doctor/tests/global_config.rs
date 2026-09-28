//! The **global** `~/.config/outl/config.toml`, not this workspace's.
//!
//! `doctor` is the tool a user runs when the app behaves oddly, and a
//! settings file that stopped parsing is exactly that (issue #284). It
//! only *phrases* the finding — `outl_config::Loaded::notice` writes the
//! sentence — so what is pinned here is that the sentence arrives, that
//! it arrives as a warning, and that a healthy config stays silent.

use super::*;

/// `outl doctor` is the tool a user runs when the app is behaving oddly,
/// and "my settings reset themselves" is exactly that (issue #284). It has
/// to name the unreadable `config.toml` — the CLI is the surface a GUI
/// user still has when neither GUI says anything at boot.
///
/// Passes the notice directly rather than writing a broken file into the
/// real `~/.config/outl/`: the doctor only phrases what
/// `outl_config::Loaded::notice` hands it, and that sentence has its own
/// test in `outl-config`. Reading the developer's own config here is the
/// process-wide coupling `collect_with_store`'s comment refuses.
#[test]
fn a_config_that_does_not_parse_is_reported_as_a_warning() {
    let (_dir, root, _paths) = fresh();
    let store_dir = TempDir::new().expect("temp device store");

    let report = super::collect_internal(
        &root,
        true,
        false,
        RepairScope::Guarded,
        &outl_core::device::DeviceStore::at(store_dir.path()),
        &outl_config::ThemeCfg::default(),
        Some("~/.config/outl/config.toml could not be read (line 7: boom)".into()),
    )
    .expect("doctor runs with an unreadable global config");

    assert!(
        has(&report, "line 7: boom"),
        "the reason has to reach the report, not just the fact: {:#?}",
        messages(&report)
    );
    // A warning, never an error: the workspace is intact and nothing
    // overwrote the file. Ranking it beside a torn op log is how the loud
    // lines in this report stop being read.
    assert_eq!(report.error_count, 0, "{:#?}", messages(&report));
    assert!(report.warn_count >= 1);
}

/// The other half: a readable config is silent. A doctor that warned on
/// every run would train the user to ignore it.
#[test]
fn a_readable_config_produces_no_finding_about_it() {
    let (_dir, root, _paths) = fresh();
    let report = collect(&root, false).expect("doctor runs");
    assert!(
        !has(&report, "could not be read"),
        "{:#?}",
        messages(&report)
    );
}
