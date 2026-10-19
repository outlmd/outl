//! `outl-tui` — the terminal UI binary.
//!
//! Thin wrapper around the library `outl_tui::run`. The `outl` binary
//! (in `outl-cli`) reuses the same library so that `outl` with no
//! subcommand opens this TUI in the current directory. See
//! `crates/outl-tui/CLAUDE.md`.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "outl-tui",
    about = "Terminal UI for the outl outliner.",
    version
)]
struct Cli {
    /// Workspace path. Defaults to the current directory.
    #[arg(default_value = ".")]
    path: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Crash reporting (RFC 0377), in the binary rather than in `outl_tui::run`
    // on purpose: `run` is also called by the `outl` binary, which has already
    // started its own reporter by the time it gets there. Initialising inside
    // the library would mean two `sentry::init` calls in one process on the
    // `outl` path, with the second replacing the first's hub.
    //
    // Named binding, not `let _ =` — the guard flushes on drop.
    let _telemetry = outl_telemetry::init("tui", outl_config::telemetry_enabled());

    outl_tui::run(&cli.path)
}
