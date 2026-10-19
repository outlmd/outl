//! `outl telemetry` — the terminal opt-out for crash reporting.
//!
//! Reporting is on by default (RFC 0377), so these three verbs are the
//! whole opt-out for anyone who never opens a GUI client. Two properties
//! are worth a test each: the flag survives the process that wrote it,
//! and `status` never answers from a default when the file on disk says
//! something it could not read.
//!
//! **Every command here runs with `XDG_CONFIG_HOME` pointed at a
//! tempdir.** `outl_config::config_dir()` honours that var ahead of
//! every other branch, so the suite never reads or writes the
//! developer's own `~/.config/outl/config.toml` — which is the isolation
//! question issue #211 was closed on (root `CLAUDE.md` invariant 9).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn outl(config_home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_outl"));
    cmd.env("XDG_CONFIG_HOME", config_home);
    cmd
}

fn telemetry(config_home: &Path, verb: &str) -> Output {
    outl(config_home)
        .args(["telemetry", verb])
        .output()
        .expect("spawning outl")
}

/// Run a verb that must succeed, returning its stdout.
fn ok(config_home: &Path, verb: &str) -> String {
    let out = telemetry(config_home, verb);
    assert!(
        out.status.success(),
        "`outl telemetry {verb}` failed:\nstatus: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn config_file(config_home: &Path) -> std::path::PathBuf {
    config_home.join("outl").join("config.toml")
}

#[test]
fn status_on_a_fresh_config_reports_the_on_default() {
    let home = TempDir::new().unwrap();

    let out = ok(home.path(), "status");

    assert!(out.contains("crash reports: on"), "{out}");
    // The promise is the reason the default is defensible at all, so it
    // travels with the state rather than living only in the docs.
    assert!(out.contains("notes"), "no note guarantee in output:\n{out}");
    assert!(
        out.contains("outl telemetry off"),
        "status must name the way out:\n{out}"
    );
}

#[test]
fn off_then_status_reports_off() {
    let home = TempDir::new().unwrap();

    let off = ok(home.path(), "off");
    assert!(off.contains("crash reports: off"), "{off}");

    let status = ok(home.path(), "status");
    assert!(status.contains("crash reports: off"), "{status}");
}

#[test]
fn off_then_on_round_trips_back_to_on() {
    let home = TempDir::new().unwrap();

    ok(home.path(), "off");
    let on = ok(home.path(), "on");
    assert!(on.contains("crash reports: on"), "{on}");

    let status = ok(home.path(), "status");
    assert!(status.contains("crash reports: on"), "{status}");
}

/// The opt-out is useless if it is per-process state.
///
/// Two halves: the flag reaches the file every client reads, and a
/// second `outl` process reads it back off disk.
#[test]
fn off_persists_to_the_config_file_a_second_process_reads() {
    let home = TempDir::new().unwrap();

    ok(home.path(), "off");

    let raw = fs::read_to_string(config_file(home.path())).expect("config.toml must be written");
    assert!(raw.contains("[telemetry]"), "{raw}");
    assert!(raw.contains("enabled = false"), "{raw}");

    // A fresh process, same config home.
    let status = ok(home.path(), "status");
    assert!(status.contains("crash reports: off"), "{status}");
}

/// A `status` that cannot read the file must not answer from the
/// default, and what it reports has to match what the clients do.
///
/// This is the test that justifies `load_result()` over `load()`:
/// `load()` discards the read verdict, so a `config.toml` holding
/// `enabled = false` behind a syntax error would print a flat
/// `crash reports: on`. Lying about the state of an opt-out is the worst
/// failure this command has.
///
/// It now pins the other half too. `outl_config::telemetry_enabled`
/// fails closed on an unreadable file, so "off" is not a hedge here — it
/// is what every client is doing. This command used to print
/// `on (the default …)`, which was accurate about the old gate and is a
/// lie about this one.
#[test]
fn status_over_an_unreadable_config_says_so_instead_of_answering() {
    let home = TempDir::new().unwrap();
    let path = config_file(home.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Unclosed table header — and the user did ask for `off`.
    fs::write(&path, "[telemetry\nenabled = false\n").unwrap();

    let out = telemetry(home.path(), "status");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        said.contains("could not be read"),
        "status must name the unreadable file:\n{said}"
    );
    assert!(
        !said.lines().any(|l| l.trim() == "crash reports: on"),
        "status reported the default as if it were the user's setting:\n{said}"
    );
    assert!(
        said.contains("crash reports: off"),
        "the gate fails closed on an unreadable config, so status must say \
         nothing is being sent rather than naming the ON default:\n{said}"
    );
    assert!(
        !out.status.success(),
        "a status that could not be determined must not exit 0:\n{said}"
    );
}
