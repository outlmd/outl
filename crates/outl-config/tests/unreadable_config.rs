//! A `config.toml` that does not parse must never be overwritten
//! (issue #284).
//!
//! `load_from` used to erase the difference between "there is no file
//! yet" and "the user's file is broken", and `save` writes the *whole*
//! struct. One bad character plus one UI toggle therefore replaced every
//! preference — theme, `vim_mode`, timezone, `[sync] transport` — with
//! defaults, and the original was gone.
//!
//! These tests pin the two halves of the fix and the message that carries
//! it: the degraded state is observable on the load, the write path
//! refuses, and the refusal names the file and the way out. **Do not
//! relax them into "save returns an error somewhere"** — the byte-for-byte
//! assertion is the whole point, because a refusal that still truncated
//! the file would pass a weaker check.

use std::fs;
use std::path::Path;

use outl_config::{Config, ConfigSource, SaveError};
use tempfile::TempDir;

/// A hand-edited config mid-typo: `[editor` never closes. Everything in
/// it is a value the user chose and would not get back.
const BROKEN: &str = "\
[theme]
preset = \"dracula\"

[sync]
transport = \"file\"

[editor
vim_mode = false
";

fn broken_config(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("config.toml");
    fs::write(&path, BROKEN).expect("seed a broken config");
    path
}

#[test]
fn a_parse_error_is_observable_on_the_load() {
    let tmp = TempDir::new().expect("tempdir");
    let path = broken_config(tmp.path());

    let loaded = outl_config::load_result_from(&path);

    assert!(
        matches!(loaded.source, ConfigSource::Unreadable(_)),
        "a parse error must survive the load, not become a log line: {:?}",
        loaded.source
    );
    assert!(
        loaded.error().is_some_and(|e| !e.is_empty()),
        "the reason has to travel with the verdict"
    );
    assert_eq!(
        loaded.config,
        Config::default(),
        "the config beside an Unreadable verdict is defaults, not the file"
    );
}

#[test]
fn the_notice_a_client_shows_names_the_path_and_the_reason() {
    let tmp = TempDir::new().expect("tempdir");
    let path = broken_config(tmp.path());

    let notice = outl_config::load_result_from(&path)
        .notice()
        .expect("an unreadable config owes the user a sentence");

    assert!(notice.contains(&path.display().to_string()), "{notice}");
    assert!(
        notice.contains("line 7"),
        "the failing line travels too: {notice}"
    );
    assert!(
        notice.contains("defaults"),
        "it has to say what the app is running on instead: {notice}"
    );
    // One owner for this sentence: the TUI status line and `outl doctor`
    // both print THIS string. A client writing its own is a second copy
    // that drifts (root CLAUDE.md invariant 12).
    assert!(
        outl_config::load_result_from(&tmp.path().join("absent.toml"))
            .notice()
            .is_none(),
        "a first launch owes the user nothing"
    );
}

#[test]
fn a_missing_file_is_a_first_launch_not_a_failure() {
    let tmp = TempDir::new().expect("tempdir");
    let loaded = outl_config::load_result_from(&tmp.path().join("config.toml"));

    assert_eq!(loaded.source, ConfigSource::Missing);
    assert!(loaded.error().is_none());
    assert_eq!(loaded.config, Config::default());
}

#[test]
fn an_empty_file_is_a_parsed_config_of_all_defaults() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("config.toml");
    fs::write(&path, "").expect("write empty");

    let loaded = outl_config::load_result_from(&path);

    assert_eq!(
        loaded.source,
        ConfigSource::Parsed,
        "every field has a serde default, so an empty file is a valid config"
    );
    assert_eq!(loaded.config, Config::default());
}

#[test]
fn a_save_over_an_unparseable_config_refuses_and_leaves_the_bytes_alone() {
    let tmp = TempDir::new().expect("tempdir");
    let path = broken_config(tmp.path());

    let err = outl_config::save_to(&path, &Config::default())
        .expect_err("saving over a config that failed to parse must refuse");

    assert!(
        matches!(err, SaveError::Unreadable { .. }),
        "the refusal has to be distinguishable from an I/O failure: {err:?}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read back"),
        BROKEN,
        "the user's file must be byte-for-byte intact after a refused save"
    );
}

#[test]
fn the_refusal_names_the_file_and_the_way_out() {
    let tmp = TempDir::new().expect("tempdir");
    let path = broken_config(tmp.path());

    let msg = outl_config::save_to(&path, &Config::default())
        .expect_err("must refuse")
        .to_string();

    assert!(
        msg.contains(&path.display().to_string()),
        "a user can only fix a file the message names: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("fix"),
        "the message has to say what to do about it: {msg}"
    );
}

#[test]
fn a_first_launch_save_creates_the_file_as_before() {
    let tmp = TempDir::new().expect("tempdir");
    // A directory that does not exist yet either — the real first launch.
    let path = tmp.path().join("nested").join("config.toml");

    let mut cfg = Config::default();
    cfg.theme.preset = "nord".into();
    outl_config::save_to(&path, &cfg).expect("a missing config is not a refusal");

    let back = outl_config::load_result_from(&path);
    assert_eq!(back.source, ConfigSource::Parsed);
    assert_eq!(back.config.theme.preset, "nord");
}

#[test]
fn a_save_over_a_parseable_config_still_overwrites_it() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("config.toml");

    let mut first = Config::default();
    first.theme.preset = "nord".into();
    outl_config::save_to(&path, &first).expect("seed");

    let mut second = Config::default();
    second.theme.preset = "dracula".into();
    outl_config::save_to(&path, &second).expect("a readable config is writable");

    assert_eq!(outl_config::load_from(&path).theme.preset, "dracula");
}
