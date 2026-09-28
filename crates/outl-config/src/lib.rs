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
//! [tui]
//! icons = "emoji"        # "emoji" (default) | "nerd-font"
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
//! [`Config::default`]. A malformed file still boots on defaults rather
//! than refusing to start; user-pickable preferences aren't worth
//! blocking the app on. What it does **not** do is pretend the file said
//! that: the load carries a [`ConfigSource`] verdict, and [`save`]
//! refuses to write over a file it could not read (issue #284). Writing
//! the whole struct back over an unreadable file is how one bad character
//! plus one UI toggle used to replace every preference with defaults.
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

mod atomic;
mod paths;
mod schema;
mod tui;

pub use paths::{config_dir, config_path};
pub use schema::{
    AssetsCfg, BacklinksOrder, BackupCfg, CalendarCfg, Config, DisplayCfg, EditorCfg, RemindersCfg,
    SnapshotCfg, StorageCfg, SyncConfig, SyncTransportKind, ThemeCfg, ThemeMode, WorkspaceCfg,
};
pub use tui::{TuiCfg, TuiIconStyle};

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where the [`Config`] handed back by a load actually came from.
///
/// Three states, because the old return type had one and the difference
/// between them is what the user needs (issue #284): a missing file is a
/// first launch, and an unreadable one is the user's own values sitting
/// on disk where nothing can see them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// Nothing at this path. First launch — the only state in which
    /// [`Config::default`] is what the user actually asked for.
    Missing,
    /// The file turned into a `Config`. An empty file lands here too:
    /// every field carries a serde default, so `""` is a valid config
    /// that means "all defaults".
    Parsed,
    /// The file is on disk and could not be turned into a `Config` — a
    /// TOML syntax error, a type mismatch, or an I/O error that is not
    /// "not found". The `Config` alongside is defaults, and it is **not**
    /// what the file says, so [`save_to`] refuses to overwrite it.
    Unreadable(String),
}

/// A [`Config`] plus the answer to "is this the user's, or ours?".
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Always usable: the parsed file, or defaults standing in for one.
    pub config: Config,
    /// Where `config` came from.
    pub source: ConfigSource,
    /// The file this verdict is about, so a client reporting the problem
    /// names the path the load actually read instead of re-deriving one.
    pub path: PathBuf,
}

impl Loaded {
    /// Why the file could not be read, when it could not. One line, so it
    /// fits a TUI status line or a toast.
    pub fn error(&self) -> Option<&str> {
        match &self.source {
            ConfigSource::Unreadable(detail) => Some(detail),
            _ => None,
        }
    }

    /// The sentence a client shows the user when the file could not be
    /// read. `None` when there is nothing to say.
    ///
    /// Owned here, not written per client. The TUI status line, `outl
    /// doctor` and any future GUI banner are three surfaces for one fact,
    /// and three copies of a sentence drift in three directions (root
    /// `CLAUDE.md` invariant 12 — "the reason text belongs in the catalog,
    /// not the client").
    ///
    /// A different sentence from [`SaveError::Unreadable`] on purpose:
    /// that one answers "why did my change not stick", this one answers
    /// "why is nothing the way I left it". Path first, because it is the
    /// one thing the user has to act on and a narrow status line truncates
    /// the tail.
    pub fn notice(&self) -> Option<String> {
        self.error().map(|detail| {
            format!(
                "{} could not be read ({detail}) — every preference is running on \
                 defaults, and settings will not save until it parses. Nothing has \
                 overwritten the file.",
                self.path.display()
            )
        })
    }

    fn unreadable(path: &Path, detail: String) -> Self {
        Self {
            config: Config::default(),
            source: ConfigSource::Unreadable(detail),
            path: path.to_path_buf(),
        }
    }
}

/// Why a [`save`] did not happen.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The file on disk could not be read, so writing the whole struct
    /// over it would replace values this process never had with defaults.
    ///
    /// The way out is the file itself — it is a hand-edited file
    /// (`docs/config.md`), it is still byte-for-byte intact, and the
    /// message names it. `outl doctor` reports the same thing.
    #[error(
        "settings not saved: {path} could not be read ({detail}). \
         Fix that file, then try again"
    )]
    Unreadable { path: PathBuf, detail: String },

    /// Creating the directory, writing the scratch file, or renaming it
    /// over `config.toml` failed.
    #[error("could not write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The `Config` itself could not be encoded as TOML.
    #[error("could not encode the config as TOML: {0}")]
    Encode(#[from] toml::ser::Error),
}

impl SaveError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Load `config.toml` from the default path, discarding the verdict.
///
/// The convenience for call sites that read one field and have nothing to
/// do about a broken file (`assets.max_bytes`, the backlinks order, the
/// boot timezone). They stay safe because the **write** path re-reads the
/// file and refuses on its own — see [`save_to`]. Anything that can tell
/// the user goes through [`load_result`] instead.
pub fn load() -> Config {
    load_from(&config_path())
}

/// Load from a specific path, discarding the verdict. Exposed mainly for
/// tests; production code should use [`load`] or [`load_result`].
pub fn load_from(path: &Path) -> Config {
    load_result_from(path).config
}

/// Load `config.toml` from the default path **with** the verdict: is this
/// the user's file, or defaults standing in for one?
///
/// Use this wherever the answer changes what the user is told. `load`
/// erased the difference, which is how a parse error became a
/// `tracing::warn!` in a log nobody opens and the user's only signal was
/// the app coming back in light mode (issue #284).
pub fn load_result() -> Loaded {
    load_result_from(&config_path())
}

/// [`load_result`] against a specific path.
///
/// The `tracing::warn!` for an unreadable file is emitted here and not in
/// the private `read_at` below, so the guard inside [`save_to`] does not
/// log the same failure a second time on every write.
pub fn load_result_from(path: &Path) -> Loaded {
    let loaded = read_at(path);
    if let Some(detail) = loaded.error() {
        tracing::warn!(
            "config {} could not be read ({detail}); using defaults",
            path.display()
        );
    }
    loaded
}

/// Read and classify, without logging. The single owner of the three-way
/// verdict: both the load and the save guard go through it, so they cannot
/// disagree about whether a file is readable.
fn read_at(path: &Path) -> Loaded {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                config: Config::default(),
                source: ConfigSource::Missing,
                path: path.to_path_buf(),
            };
        }
        // Anything else — a permission denial, a directory in the way, a
        // half-written file from another device — is a file whose contents
        // we do not have. `Unreadable`, so the write path refuses instead
        // of replacing whatever is there with defaults.
        Err(e) => return Loaded::unreadable(path, e.to_string()),
    };
    match toml::from_str::<Config>(&raw) {
        Ok(config) => Loaded {
            config,
            source: ConfigSource::Parsed,
            path: path.to_path_buf(),
        },
        Err(e) => Loaded::unreadable(path, one_line(&raw, &e)),
    }
}

/// A TOML parse failure squeezed into one line, of the shape
/// `line 4: unclosed table, expected ]`.
///
/// `Display` on a `toml::de::Error` is a four-line snippet with a caret.
/// That is right for a terminal and wrong for a status line or a toast,
/// and this text is what the user reads.
fn one_line(raw: &str, e: &toml::de::Error) -> String {
    let msg = e.message().replace('\n', "; ");
    match e.span().and_then(|s| raw.get(..s.start)) {
        Some(before) => {
            let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
            format!("line {line}: {msg}")
        }
        None => msg,
    }
}

/// Save `config` to the default path atomically (hidden scratch file →
/// `config.toml` rename). Creates `~/.config/outl/` if missing.
///
/// Refuses when the file on disk does not parse — see [`save_to`].
pub fn save(config: &Config) -> Result<(), SaveError> {
    save_to(&config_path(), config)
}

/// Save to a specific path. Exposed mainly for tests.
///
/// **Refuses to write over a config it cannot read.** Every caller does
/// `load()` → mutate one field → `save()`, and `save` writes the whole
/// struct, so writing over a file that failed to parse replaces the
/// user's theme, `vim_mode`, timezone and `[sync] transport` with
/// defaults — one bad character plus one UI toggle and the original is
/// gone (issue #284). The check lives here rather than in the callers
/// because a caller that loaded hours ago, or never loaded at all, cannot
/// answer it. There is no `force` variant: the escape hatch is the file,
/// which is hand-editable by design and still intact.
///
/// Publishes by rename: write a hidden sibling scratch file, `fsync` it,
/// `rename` it over `path`, then `fsync` the parent directory so the
/// rename itself is durable. Both `fsync`s matter for this file — a
/// config that comes back zero-length after a power loss silently resets
/// the user to defaults (theme, vim mode, last workspace), and the old
/// code had neither.
///
/// **The scratch name is unique per call** — see `atomic::tmp_path`. A shared
/// one made concurrent saves publish a zero-byte config, which this
/// crate reads back as a valid config of all defaults.
///
/// The scratch file is owned by a `TempFile` guard, so it is unlinked
/// on every in-process exit path instead of being left next to the real
/// config on the first transient error. What a *killed* process leaves
/// behind is swept by `atomic::sweep_stale_scratch`.
pub fn save_to(path: &Path, config: &Config) -> Result<(), SaveError> {
    if let ConfigSource::Unreadable(detail) = read_at(path).source {
        return Err(SaveError::Unreadable {
            path: path.to_path_buf(),
            detail,
        });
    }

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| SaveError::io(dir, e))?;
    }
    atomic::publish(path, &toml::to_string_pretty(config)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic::test_support::leftover_temps;
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

    /// The guard, reached through the API a client actually calls.
    ///
    /// The byte-for-byte assertions live in
    /// `tests/unreadable_config.rs`; this one pins that a directory in
    /// place of the file is classified the same way as a syntax error —
    /// unreadable is "we do not have these bytes", not "the TOML is bad".
    #[test]
    fn a_config_we_cannot_even_read_refuses_the_save() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        fs::create_dir(&path).unwrap();

        let err = save_to(&path, &Config::default()).expect_err("must refuse");
        assert!(matches!(err, SaveError::Unreadable { .. }), "{err:?}");
        assert_eq!(leftover_temps(tmp.path()), Vec::<PathBuf>::new());
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
