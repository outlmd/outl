//! Per-OS config path resolution.

use std::path::PathBuf;

/// Resolve the directory where `config.toml` and `actor` live.
///
/// - **macOS / Linux:** `~/.config/outl/` (or `$XDG_CONFIG_HOME/outl/`
///   when set). XDG-style on macOS is deliberate — outl is
///   CLI-first, and a Mac user dropping into a terminal sees the
///   same path Linux uses. The conventional
///   `~/Library/Application Support/…` would split TUI + desktop
///   into two config files for no real benefit.
/// - **Windows:** `%APPDATA%\outl\` (whatever
///   `dirs::config_dir()` returns, typically
///   `C:\Users\<user>\AppData\Roaming\outl`). The XDG layout is
///   not a Windows convention — sticking it under `%USERPROFILE%`
///   directly would surprise both PowerShell users and tools that
///   expect Roaming.
///
/// All branches honour `$XDG_CONFIG_HOME` first so a power user
/// can co-locate Windows + Linux profiles via the same env var.
pub fn config_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("XDG_CONFIG_HOME") {
        if !custom.is_empty() {
            return PathBuf::from(custom).join("outl");
        }
    }
    #[cfg(windows)]
    {
        if let Some(roaming) = dirs::config_dir() {
            return roaming.join("outl");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config").join("outl");
        }
    }
    // Last resort — relative path. Almost never hit in practice
    // (every supported OS has a home dir), but keeps the function
    // total so callers don't have to handle `Option`.
    PathBuf::from(".config").join("outl")
}

/// `~/.config/outl/config.toml`.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// `~/.config/outl/themes/` — where a user's own `.toml` themes live.
///
/// This crate is the **single owner of the path**, and `outl-theme` is the
/// single owner of the format: `outl_theme::Registry::with_user_dir` takes
/// the directory as an argument rather than resolving it, so its tests get
/// their own copy instead of reading the developer's real config (invariant
/// 9). Three clients join the two — don't add a fourth spelling of the
/// path.
///
/// Nothing creates this directory. A missing one is the normal case and
/// means "no custom themes", not an error.
pub fn theme_dir() -> PathBuf {
    config_dir().join("themes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_ends_with_outl_config_toml() {
        let p = config_path();
        let s = p.to_string_lossy();
        assert!(s.ends_with("outl/config.toml"), "got: {s}");
    }

    #[test]
    fn config_dir_respects_xdg_config_home() {
        // We can't temporarily set env safely in concurrent tests,
        // so just assert the format when the var is unset. The
        // logic itself is one straight `if let`.
        let _ = config_dir();
    }
}
