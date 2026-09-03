//! # outl-config
//!
//! Shared user-config for every outl client.
//! The whole point of this crate is one file in one place:
//!
//! ```text
//! ~/.config/outl/config.toml
//! ```
//!
//! The path is **XDG-style even on macOS** — not the platform-native
//! `~/Library/Application Support/…`. outl is keyboard-first and
//! CLI-friendly, and a Mac user dropping into a terminal sees the
//! same `~/.config/outl/config.toml` they'd see on Linux. The TUI
//! and the desktop app read and write the same file; sharing this
//! crate is what guarantees they agree on the schema.
//!
//! ## Schema
//!
//! ```toml
//! [workspace]
//! last = "/Users/me/iCloud/outl"
//!
//! [theme]
//! preset = "outl"        # name from outl_theme::PRESETS; the light side of the pair
//! preset_dark = "dracula"   # optional; dark side. Omit = falls back to `preset`
//! mode = "auto"          # "light" | "dark" | "auto" (default)
//!
//! [editor]
//! vim_mode = true
//! font_size = 15
//!
//! [calendar]
//! timezone = "Europe/London"   # IANA name; omit = OS local timezone
//!
//! [sync]
//! transport = "iroh"   # "iroh" (P2P, default) | "file" (iCloud/fs opt-out)
//! relay_url = ""        # optional; empty = outl's default relay (use1-1.relay.avelino.outl.iroh.link)
//!
//! [snapshot]
//! enabled = true        # default; long-lived clients write a snapshot periodically
//! op_threshold = 10000  # write after this many applied ops
//!
//! [display]
//! backlinks_order = "newest"   # "newest" (default) | "oldest"
//! ```
//!
//! All fields are optional — missing values fall back to
//! [`Config::default`]. A malformed file is logged and replaced with
//! defaults rather than refused to boot; user-pickable preferences
//! aren't worth blocking the app on.
//!
//! ## What goes in here vs the op log
//!
//! - **In here**: local-only preferences (vim mode, theme, font size,
//!   last opened workspace path).
//! - **In the op log** (`ops-*.jsonl`): anything that must converge
//!   between devices. Block content, collapsed flags, properties.
//!
//! See the root `CLAUDE.md` invariant #7 — "any state that must
//! converge between devices goes through the op log".

mod paths;
mod schema;

pub use paths::{config_dir, config_path};
pub use schema::{
    AssetsCfg, BacklinksOrder, BackupCfg, CalendarCfg, Config, DisplayCfg, EditorCfg, RemindersCfg,
    SnapshotCfg, StorageCfg, SyncConfig, SyncTransportKind, ThemeCfg, ThemeMode, TuiCfg,
    WorkspaceCfg,
};

use std::fs;
use std::path::Path;

/// Load `config.toml` from the default path. Returns
/// [`Config::default`] when the file doesn't exist (first launch),
/// is empty, or fails to parse — all three are recoverable user
/// states, not errors worth surfacing.
pub fn load() -> Config {
    load_from(&config_path())
}

/// Load from a specific path. Exposed mainly for tests; production
/// code should always use [`load`].
pub fn load_from(path: &Path) -> Config {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Config::default(),
    };
    match toml::from_str::<Config>(&raw) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "config {} parse error ({e}); using defaults",
                path.display()
            );
            Config::default()
        }
    }
}

/// Save `config` to the default path atomically (`config.toml.tmp`
/// → `config.toml` rename). Creates `~/.config/outl/` if missing.
pub fn save(config: &Config) -> anyhow::Result<()> {
    save_to(&config_path(), config)
}

/// Save to a specific path. Exposed mainly for tests.
pub fn save_to(path: &Path, config: &Config) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    let body = toml::to_string_pretty(config)?;
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn load_returns_defaults_when_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nope.toml");
        let cfg = load_from(&path);
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.workspace.last = Some(PathBuf::from("/tmp/ws"));
        cfg.theme.preset = "dracula".into();
        cfg.editor.vim_mode = false;
        cfg.editor.font_size = 18;

        save_to(&path, &cfg).unwrap();
        let back = load_from(&path);
        assert_eq!(back.workspace.last, Some(PathBuf::from("/tmp/ws")));
        assert_eq!(back.theme.preset, "dracula");
        assert!(!back.editor.vim_mode);
        assert_eq!(back.editor.font_size, 18);
    }

    #[test]
    fn load_falls_back_on_corrupted_toml() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.toml");
        fs::write(&path, "[unclosed").unwrap();
        let cfg = load_from(&path);
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_toml_uses_field_defaults() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("partial.toml");
        fs::write(
            &path,
            r#"
[theme]
preset = "nord"
"#,
        )
        .unwrap();
        let cfg = load_from(&path);
        assert_eq!(cfg.theme.preset, "nord");
        // Editor + workspace fall back to defaults.
        assert!(cfg.editor.vim_mode);
        assert_eq!(cfg.editor.font_size, 15);
        assert!(cfg.workspace.last.is_none());
    }
}
