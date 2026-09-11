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
//! preset = "outl-light"  # name from outl_theme::PRESETS; the light side of the pair
//! preset_dark = "outl"   # optional; dark side. Omit = falls back to `preset`
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
use std::path::{Path, PathBuf};

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

/// Save `config` to the default path atomically (hidden scratch file →
/// `config.toml` rename). Creates `~/.config/outl/` if missing.
pub fn save(config: &Config) -> anyhow::Result<()> {
    save_to(&config_path(), config)
}

/// Save to a specific path. Exposed mainly for tests.
///
/// Publishes by rename: write a hidden sibling scratch file, `fsync` it,
/// `rename` it over `path`, then `fsync` the parent directory so the
/// rename itself is durable. Both `fsync`s matter for this file — a
/// config that comes back zero-length after a power loss silently resets
/// the user to defaults (theme, vim mode, last workspace), and the old
/// code had neither.
///
/// The scratch file is owned by a `TempFile` guard, so it is unlinked
/// on every in-process exit path instead of being left next to the real
/// config as `config.toml.tmp` on the first transient error.
pub fn save_to(path: &Path, config: &Config) -> anyhow::Result<()> {
    use std::io::Write as _;

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = toml::to_string_pretty(config)?;
    let guard = TempFile::new(tmp_path(path));

    {
        let mut file = fs::File::create(guard.path())?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(guard.path(), path)?;
    guard.keep();

    // Best-effort: Windows refuses to open a directory as a file, and
    // failing the whole save there would be worse than the durability
    // gap being closed.
    if let Some(dir) = path.parent() {
        if let Ok(handle) = fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    Ok(())
}

/// Scratch path for an in-flight rewrite of `path`: the filename with a
/// leading dot and a `.tmp` suffix, so `config.toml` becomes
/// `.config.toml.tmp`.
fn tmp_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default();
    let mut scratch = std::ffi::OsString::from(if name.as_encoded_bytes().starts_with(b".") {
        ""
    } else {
        "."
    });
    scratch.push(name);
    scratch.push(".tmp");
    path.with_file_name(scratch)
}

/// A temp file that deletes itself unless [`TempFile::keep`] is called.
///
/// **This is a deliberate third copy, and it is the only one that had no
/// alternative.** The canonical guard is `outl_md::atomic::TempFile`
/// (which `outl-actions` and `outl-sync-iroh` both use); a second,
/// module-private one lives in `outl_core::storage::sidecar`. This crate
/// is a leaf — it depends on no other `outl-*` crate, and inverting that
/// so a config parser pulls in the markdown pipeline (comrak) and the
/// CRDT kernel (yrs) to reuse twenty lines is a worse trade than the
/// duplication.
///
/// Keep the three in sync by hand, and prefer the `outl-md` one for any
/// new call site that can reach it. If this crate ever gains an
/// `outl-md` edge for another reason, delete this copy.
struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// The rename succeeded; there is nothing left at this path.
    fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
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

    /// Every `*.tmp` sibling in `dir`, whatever it is called.
    fn leftover_temps(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<_> = fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "tmp"))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn a_successful_save_leaves_no_scratch_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        save_to(&path, &Config::default()).unwrap();
        assert_eq!(leftover_temps(tmp.path()), Vec::<PathBuf>::new());
    }

    /// The failure path: a real I/O error, no injection — renaming onto a
    /// non-empty directory fails on every platform we ship. The old code
    /// cleaned up on no failure path at all, leaving `config.toml.tmp` in
    /// `~/.config/outl/` after any transient error.
    #[test]
    fn a_failed_save_leaves_no_scratch_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("occupant"), b"x").unwrap();

        save_to(&path, &Config::default()).expect_err("rename onto a non-empty dir must fail");
        assert_eq!(
            leftover_temps(tmp.path()),
            Vec::<PathBuf>::new(),
            "a failed config save must not leak its scratch file"
        );
    }

    /// A failed save must not damage the config already on disk — the
    /// whole reason this publishes by rename.
    #[test]
    fn a_failed_save_leaves_the_previous_config_intact() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.theme.preset = "dracula".into();
        save_to(&path, &cfg).unwrap();

        // Occupy the scratch path with a directory so `File::create` fails.
        fs::create_dir(tmp_path(&path)).unwrap();
        save_to(&path, &Config::default()).expect_err("a blocked scratch path must fail the save");

        assert_eq!(load_from(&path).theme.preset, "dracula");
    }

    #[test]
    fn the_scratch_file_is_a_dotfile() {
        assert_eq!(
            tmp_path(Path::new("/x/config.toml")).file_name().unwrap(),
            ".config.toml.tmp"
        );
    }

    #[test]
    fn the_guard_unlinks_an_unkept_temp() {
        let tmp = TempDir::new().unwrap();
        let scratch = tmp.path().join(".scratch.tmp");
        fs::write(&scratch, b"half").unwrap();
        drop(TempFile::new(scratch.clone()));
        assert!(!scratch.exists(), "dropping an armed guard must unlink");
    }

    #[test]
    fn the_guard_leaves_a_kept_temp_alone() {
        let tmp = TempDir::new().unwrap();
        let scratch = tmp.path().join(".scratch.tmp");
        fs::write(&scratch, b"published").unwrap();
        TempFile::new(scratch.clone()).keep();
        assert!(scratch.exists(), "keep() must disarm the unlink");
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
