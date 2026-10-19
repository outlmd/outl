//! `outl telemetry` — the terminal switch for crash reporting.
//!
//! Reporting is **on by default** ([RFC 0377]), so this command is the
//! entire opt-out for anyone who never opens a GUI client. The TUI has
//! no settings screen, which is why `outl_shortcuts::capability_support`
//! declares `CrashReportingOptOut` as `Missing` there and names these
//! verbs in the nudge.
//!
//! Reads and writes one boolean in `~/.config/outl/config.toml`
//! (`outl_config`), the same file the desktop and mobile Privacy
//! sections write. Deliberately **not** a generic `outl config set
//! <key> <value>`: that needs a schema story for every key in `Config`,
//! which is a much larger change arriving as a side effect of this one.
//!
//! Nothing here initialises or inspects the reporter — the four
//! conditions an event has to clear live in `outl-telemetry`. This file
//! only owns condition 3, the user's answer.
//!
//! [RFC 0377]: ../../../../docs/rfcs/0377-crash-reporting.md

use anyhow::{Context, Result};
use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum TelemetrySubcommand {
    /// Show whether crash reports are being sent from this device.
    Status,
    /// Start sending crash reports.
    On,
    /// Stop sending crash reports.
    Off,
}

pub fn run(sub: &TelemetrySubcommand) -> Result<()> {
    match sub {
        TelemetrySubcommand::Status => status(),
        TelemetrySubcommand::On => set(true),
        TelemetrySubcommand::Off => set(false),
    }
}

/// Report the flag, and refuse to answer from a default when the file on
/// disk holds something we could not read.
///
/// `load_result`, never `load`: `load` discards the read verdict, so a
/// `config.toml` carrying `enabled = false` behind a syntax error would
/// print a flat `crash reports: on`. Misreporting the state of an
/// opt-out is the worst failure this command has — worse than failing,
/// because the user stops looking.
fn status() -> Result<()> {
    let loaded = outl_config::load_result();

    let Some(notice) = loaded.notice() else {
        print_state(loaded.config.telemetry.enabled);
        println!("config: {}", loaded.path.display());
        return Ok(());
    };

    // Off, and that is not a guess: `outl_config::telemetry_enabled`
    // fails closed on a file it could not read, so this is what every
    // client is actually doing right now. The non-zero exit is still
    // right — the user's recorded answer is unknown, and a setting they
    // cannot save is a problem whether or not it currently errs safe.
    println!("crash reports: off (your setting could not be read, so nothing is sent)");
    Err(anyhow::anyhow!(notice))
}

fn set(enabled: bool) -> Result<()> {
    let mut cfg = outl_config::load();
    cfg.telemetry.enabled = enabled;
    // `save` is atomic and refuses to write over a config it could not
    // read, which would replace every other preference with a default.
    // Propagate it: a user who typed `off`, saw no error and got exit 0
    // would believe reporting had stopped.
    outl_config::save(&cfg).context("saving the crash-reporting setting")?;
    print_state(enabled);
    Ok(())
}

/// The one place the state is phrased, so `status`, `on` and `off`
/// cannot describe the same flag differently.
fn print_state(enabled: bool) {
    if enabled {
        println!("crash reports: on");
        println!("a crash sends what broke: the error, where in the code it happened,");
        println!("your OS and the app version");
        println!("your notes are never part of it — no page titles, no block text,");
        println!("no file paths, and no account, because there isn't one");
        println!("turn it off with `outl telemetry off`");
    } else {
        println!("crash reports: off");
        println!("nothing leaves this device — a crash stays here");
        println!("turn it back on with `outl telemetry on`");
    }
}
