//! Path resolution, workspace bootstrap, and tracing setup.
//!
//! The helpers `main` needs before it can dispatch anything.

use crate::cmd;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Resolve which workspace path to operate on.
///
/// Precedence (first hit wins):
///
/// 1. **Subcommand-positional** path (`outl page get … <path>`).
/// 2. **Global `--workspace <DIR>`** flag.
/// 3. **`workspace.last`** from `~/.config/outl/config.toml`
///    (the same file the desktop's Settings modal writes when the
///    user picks a workspace — so `outl` with no args lands on the
///    workspace the user last opened in the GUI, no `--workspace`
///    flag needed).
/// 4. **Current directory** — final fallback (matches the
///    `cd ~/notes && outl` muscle memory).
///
/// A path stored in `config.toml` that no longer exists on disk is
/// skipped silently rather than failing the launch — the user
/// likely deleted / unmounted the folder and would be surprised by
/// a crash. The cwd fallback picks up.
/// Best-effort device label for `outl peer pair` when `--name` is omitted.
///
/// Shells out to the `hostname` command (present on macOS + Linux) so the
/// peer's device list reads "macbook" instead of a node-id stub, trimming the
/// macOS `.local` suffix. Returns `None` if the command is unavailable or
/// empty — pairing then advertises no alias, exactly as before this flag.
/// Kept dependency-free on purpose; `--name` is the explicit override.
pub(crate) fn default_device_name() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    let raw = String::from_utf8(out.stdout).ok()?;
    let name = raw.trim();
    let name = name.strip_suffix(".local").unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

pub(crate) fn resolve_path(global: Option<&PathBuf>, local: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = local {
        return Ok(p.clone());
    }
    if let Some(p) = global {
        return Ok(p.clone());
    }
    if let Some(p) = outl_config::load().workspace.last {
        if p.exists() {
            return Ok(p);
        }
        tracing::warn!(
            "config.toml workspace.last = {} is no longer on disk; falling back to cwd",
            p.display()
        );
    }
    std::env::current_dir().with_context(|| "reading current directory")
}

/// If `path` has no `.outl/` directory yet, prompt the user for
/// permission to initialize one. If they say no, error out cleanly.
///
/// When stdin isn't a TTY (e.g. piped) we don't prompt — instead we
/// error with the same "run `outl init`" message we used before. This
/// keeps scripted callers predictable.
pub(crate) fn ensure_workspace_or_prompt(path: &Path) -> Result<()> {
    let outl_dir = path.join(".outl");
    if outl_dir.exists() {
        return Ok(());
    }

    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        anyhow::bail!(
            "no outl workspace at {} — run `outl init {}` first",
            path.display(),
            path.display()
        );
    }

    use std::io::{BufRead, Write};
    eprintln!("No outl workspace at {}.", path.display());
    eprint!("Initialize a new workspace here? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .with_context(|| "reading prompt response")?;
    let answer = line.trim().to_lowercase();
    if answer == "y" || answer == "yes" {
        cmd::init::run(path, "global", false)?;
        Ok(())
    } else {
        anyhow::bail!("aborted — no workspace initialized at {}", path.display());
    }
}

/// Same as [`resolve_path`] but errors out when neither flag nor positional
/// was given (init refuses to create a workspace at the cwd by accident).
pub(crate) fn resolve_init_path(
    global: Option<&PathBuf>,
    local: Option<&PathBuf>,
) -> Result<PathBuf> {
    match local.or(global) {
        Some(p) => Ok(p.clone()),
        None => Err(anyhow::anyhow!(
            "`outl init` needs an explicit path: pass a positional argument or `--workspace <DIR>`"
        )),
    }
}

pub(crate) fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        // Logs MUST go to stderr; stdout carries the JSON envelope
        // that scripts/tests parse. Without this, every `INFO` line
        // from `JsonlStorage::reload` corrupts the response.
        .with_writer(std::io::stderr)
        .try_init();
}
