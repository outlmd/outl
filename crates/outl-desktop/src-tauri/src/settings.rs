//! Wire-format adapter between the Solid frontend and the shared
//! [`outl_config::Config`] file at `~/.config/outl/config.toml`.
//!
//! The frontend continues to see a flat shape (`last_workspace`,
//! `vim_mode`, `theme`, `font_size`) because that's what the
//! `SettingsModal` was built around and there's no value in
//! reshuffling the JSON wire mid-flight. Internally we convert to /
//! from the structured `Config` so the on-disk file stays human-
//! editable and the TUI can read the same source of truth.

use serde::{Deserialize, Serialize};

use outl_config::{
    BacklinksOrder, Config, DisplayCfg, EditorCfg, SyncConfig, SyncTransportKind, ThemeCfg,
    WorkspaceCfg,
};

/// Parse the flat wire string into a transport kind. Anything that isn't an
/// explicit `"file"` opt-out (including an empty string from an older frontend)
/// resolves to iroh — P2P is the default.
fn parse_transport(s: &str) -> SyncTransportKind {
    match s {
        "file" => SyncTransportKind::File,
        _ => SyncTransportKind::Iroh,
    }
}

/// Render a transport kind to the lowercase wire string the frontend uses.
fn transport_str(t: SyncTransportKind) -> String {
    match t {
        SyncTransportKind::File => "file",
        SyncTransportKind::Iroh => "iroh",
    }
    .to_string()
}

/// Parse the flat wire string into a backlinks order. Anything that isn't an
/// explicit `"oldest"` resolves to newest — the product default (issue #142).
fn parse_backlinks_order(s: &str) -> BacklinksOrder {
    match s {
        "oldest" => BacklinksOrder::Oldest,
        _ => BacklinksOrder::Newest,
    }
}

/// Render a backlinks order to the lowercase wire string the frontend uses.
fn backlinks_order_str(o: BacklinksOrder) -> String {
    match o {
        BacklinksOrder::Newest => "newest",
        BacklinksOrder::Oldest => "oldest",
    }
    .to_string()
}

/// Parse the flat wire string into a theme mode. Anything that isn't an
/// explicit `"light"` or `"dark"` resolves to `Auto` — matches
/// `ThemeMode::default()`, so an empty/unknown string from an older
/// frontend keeps today's behaviour.
fn parse_theme_mode(s: &str) -> outl_config::ThemeMode {
    match s {
        "light" => outl_config::ThemeMode::Light,
        "dark" => outl_config::ThemeMode::Dark,
        _ => outl_config::ThemeMode::Auto,
    }
}

/// Render a theme mode to the lowercase wire string the frontend uses.
fn theme_mode_str(m: outl_config::ThemeMode) -> String {
    match m {
        outl_config::ThemeMode::Light => "light",
        outl_config::ThemeMode::Dark => "dark",
        outl_config::ThemeMode::Auto => "auto",
    }
    .to_string()
}

/// Flat shape the Solid frontend's `Settings` interface
/// (`crates/outl-desktop/src/lib/api.ts`) expects. Matches what
/// `SettingsModal.tsx` reads and writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub last_workspace: Option<std::path::PathBuf>,
    /// Defaults to `true` — outl is keyboard-first and the same
    /// behaviour ships in the TUI.
    pub vim_mode: bool,
    /// Palette preset name from `outl_theme::PRESETS`. Default
    /// `"outl-light"`. The light side of the RFC 0022 pair, and the
    /// only side used when `theme_mode == "light"`.
    pub theme: String,
    /// The dark side of the RFC 0022 pair, used when `theme_mode ==
    /// "dark"` or `"auto"` resolves dark. Always a concrete preset
    /// name here (never empty) — `From<Config>` resolves it through
    /// `ThemeCfg::dark()`, which already falls back to `theme` for a
    /// config that never set a second preset.
    pub theme_dark: String,
    /// Which side of the pair to render: `"light"`, `"dark"`, or
    /// `"auto"` (default) to follow the OS appearance setting.
    pub theme_mode: String,
    /// Outline font size in pixels.
    pub font_size: u32,
    /// Sync transport: `"iroh"` (P2P, default) or `"file"` (iCloud /
    /// shared filesystem opt-out). The Sync panel writes this.
    pub sync_transport: String,
    /// Backlinks list direction: `"newest"` (default) or `"oldest"`
    /// (issue #142). Read-only here — the backlinks toggle writes it via
    /// the dedicated `set_backlinks_order` command, and `save` restores
    /// it from disk so a settings-modal write can't clobber it.
    pub backlinks_order: String,
    /// Whether this device registers OS notifications for `remind::`
    /// rules. Defaults **on**: writing `remind::` on a block is
    /// already the opt-in, and a device with no rules never fires (so
    /// it never prompts for permission either). Off keeps the rules
    /// tracked and listed without ever interrupting.
    pub reminders_enabled: bool,
    /// Quiet-hours window as `"22:00-07:00"`, or `""` for none. A fire
    /// landing inside it is pushed to the window's end.
    pub reminders_quiet_hours: String,
}

impl Settings {
    /// Default values used when `config.toml` doesn't exist yet.
    /// Mirrors `Config::default()` field-for-field.
    pub fn fresh() -> Self {
        Config::default().into()
    }
}

impl From<Config> for Settings {
    fn from(c: Config) -> Self {
        // Resolve `dark()` (never the raw `Option`) and read `mode`
        // before moving `preset` out of `c.theme` below — both borrow
        // `c.theme` immutably and `dark()`'s fallback needs `preset`
        // still in place.
        let theme_dark = c.theme.dark().to_string();
        let theme_mode = theme_mode_str(c.theme.mode);
        Self {
            last_workspace: c.workspace.last,
            vim_mode: c.editor.vim_mode,
            theme: c.theme.preset,
            theme_dark,
            theme_mode,
            font_size: c.editor.font_size,
            sync_transport: transport_str(c.sync.transport),
            backlinks_order: backlinks_order_str(c.display.backlinks_order),
            reminders_enabled: c.reminders.enabled,
            reminders_quiet_hours: c.reminders.quiet_hours.unwrap_or_default(),
        }
    }
}

impl From<Settings> for Config {
    fn from(s: Settings) -> Self {
        Self {
            workspace: WorkspaceCfg {
                last: s.last_workspace,
            },
            theme: ThemeCfg {
                preset: s.theme,
                // Always `Some`, never restored back to `None`: the
                // modal now owns the whole pair, and `Settings::theme_dark`
                // is always a concrete preset name (see its doc comment).
                // A config that only ever had `preset` therefore gets an
                // explicit `preset_dark` equal to it on the first modal
                // save, rather than staying implicit — a one-time,
                // behaviour-preserving rewrite of the file: `dark()`
                // already resolved to the same value either way, so
                // nothing the client renders changes. The alternative
                // (writing `None` back when `theme_dark == theme`) would
                // require guessing whether the user meant to pin the pair
                // or just hadn't touched it, which is exactly the
                // ambiguity this DTO change removes.
                preset_dark: Some(s.theme_dark),
                mode: parse_theme_mode(&s.theme_mode),
            },
            editor: EditorCfg {
                vim_mode: s.vim_mode,
                font_size: s.font_size,
            },
            // The flat desktop Settings doesn't model `[calendar]`; `save`
            // restores it from disk so a hand-set timezone survives a
            // settings write (same pattern as `sync.relay_url`).
            calendar: outl_config::CalendarCfg::default(),
            sync: SyncConfig {
                transport: parse_transport(&s.sync_transport),
                // relay_url isn't modeled in the flat Settings; `save` restores
                // the on-disk value so editing the transport doesn't drop it.
                relay_url: None,
            },
            // `[tui]` is TUI-only; the desktop doesn't model it. `save`
            // restores it from disk so a hand-set `mouse_capture` survives
            // a settings write (same pattern as `[calendar]`).
            tui: outl_config::TuiCfg::default(),
            // `[snapshot]` is core-managed; the desktop doesn't model it.
            // `save` restores it from disk so a hand-set policy survives a
            // settings write (same pattern as `[calendar]` / `[tui]`).
            snapshot: outl_config::SnapshotCfg::default(),
            // `[storage]` is core-managed (LRU cap for JsonlStorage);
            // same restore-on-save pattern as the other core sections.
            storage: outl_config::StorageCfg::default(),
            // `[display]` (backlinks order) is written by the dedicated
            // `set_backlinks_order` command, not the settings modal. `save`
            // restores it from disk so a modal write doesn't clobber the
            // toggle (same restore-on-save pattern as `[calendar]`).
            display: DisplayCfg {
                backlinks_order: parse_backlinks_order(&s.backlinks_order),
            },
            // `[assets]` (upload size cap) is core-managed; the desktop
            // doesn't model it. `save` restores it from disk so a
            // hand-set `max_bytes` survives a settings write (same
            // restore-on-save pattern as `[calendar]` / `[tui]`).
            assets: outl_config::AssetsCfg::default(),
            // `[backup]` is not modelled in the flat Settings either.
            // `save` restores it from disk so a settings write can never
            // silently turn a user's backups off (same restore-on-save
            // pattern as `[calendar]` / `[assets]`).
            backup: outl_config::BackupCfg::default(),
            // `[reminders]` IS modelled here — the feature is opt-in and
            // the settings modal is where the user turns it on, so
            // unlike `[calendar]` / `[tui]` it must not be restored
            // from disk on save (that would make the toggle inert).
            reminders: outl_config::RemindersCfg {
                enabled: s.reminders_enabled,
                quiet_hours: Some(s.reminders_quiet_hours).filter(|q| !q.trim().is_empty()),
            },
        }
    }
}

/// Load `config.toml` from `~/.config/outl/` and project to the
/// flat wire shape. Missing / malformed file = defaults — the
/// `outl-config` crate already logs the parse error.
///
/// The `_app_config_dir` parameter is kept for the AppState
/// signature (other modules read it for the actor file location)
/// but the config itself ignores it; the path is XDG-driven.
pub fn load(_app_config_dir: &std::path::Path) -> Settings {
    outl_config::load().into()
}

/// Overwrite the sections of `cfg` that the flat `Settings` wire shape
/// doesn't model with whatever is currently on disk (`on_disk`), so a
/// settings-modal save can only ever touch the fields it actually owns.
///
/// Split out from [`save`] as a pure function (no I/O) so the
/// restore-on-save contract for each section — including the
/// `[theme]` pair added in RFC 0022 — can be pinned by a unit test
/// without touching the real `~/.config/outl/config.toml`.
fn restore_unmodeled_sections(cfg: &mut Config, on_disk: &Config) {
    // The flat `Settings` carries the transport choice (the Sync panel
    // writes it), so `into()` already set `cfg.sync.transport`. It does NOT
    // model `relay_url` or `[calendar]`, so restore those from disk in one
    // read — otherwise saving the transport would wipe a custom relay or a
    // hand-set timezone (and two reads could mix fields across a concurrent
    // edit).
    cfg.sync.relay_url = on_disk.sync.relay_url.clone();
    cfg.calendar = on_disk.calendar.clone();
    // `[theme]` (all three fields: `preset`, `preset_dark`, `mode`) is now
    // FULLY modeled in `Settings` — the modal owns the whole pair. Do NOT
    // add a restore-from-disk line for any of them here: that was the
    // previous shape (`preset_dark` / `mode` unmodeled), and restoring them
    // now would silently overwrite whatever the user just picked in the
    // modal with the stale on-disk pair. See `settings.rs`'s theme-pair
    // test for the regression this guards against.
    // The backlinks toggle owns `[display]` via `set_backlinks_order`;
    // restore it so a modal save can't revert the user's direction.
    cfg.display = on_disk.display.clone();
    // `[assets]` (upload size cap) is core-managed and not modeled in the
    // flat Settings; restore it so a modal save keeps a custom max_bytes.
    cfg.assets = on_disk.assets.clone();
    // `[backup]` is core-managed and not modeled in the flat Settings.
    // Restoring it matters more than the others: dropping to the default
    // here would be a *silent* change to the user's safety net, and the
    // only way they'd find out is the day they needed it.
    cfg.backup = on_disk.backup.clone();
}

/// Save the flat wire shape as `config.toml`. Same path
/// (`~/.config/outl/config.toml`) regardless of where the OS
/// thinks the app's config directory is.
pub fn save(_app_config_dir: &std::path::Path, settings: &Settings) -> anyhow::Result<()> {
    let mut cfg: Config = settings.clone().into();
    let on_disk = outl_config::load();
    restore_unmodeled_sections(&mut cfg, &on_disk);
    outl_config::save(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fresh_matches_config_defaults() {
        let s = Settings::fresh();
        assert!(s.last_workspace.is_none());
        assert!(
            s.vim_mode,
            "vim mode is on by default — outl is keyboard-first"
        );
        assert_eq!(s.theme, "outl-light");
        assert_eq!(s.theme_dark, "outl");
        assert_eq!(s.theme_mode, "auto", "auto is ThemeMode::default()");
        assert_eq!(s.font_size, 15);
        assert_eq!(s.sync_transport, "iroh", "P2P is the default transport");
        assert_eq!(s.backlinks_order, "newest", "newest-first is the default");
        assert!(
            s.reminders_enabled,
            "`remind::` on a block is the opt-in; the switch defaults on"
        );
        assert_eq!(s.reminders_quiet_hours, "");
    }

    #[test]
    fn round_trips_via_config() {
        let s = Settings {
            last_workspace: Some(PathBuf::from("/tmp/ws")),
            vim_mode: false,
            theme: "dracula".into(),
            theme_dark: "nord".into(),
            theme_mode: "dark".into(),
            font_size: 18,
            sync_transport: "file".into(),
            backlinks_order: "oldest".into(),
            reminders_enabled: true,
            reminders_quiet_hours: "22:00-07:00".into(),
        };
        let cfg: Config = s.clone().into();
        let back: Settings = cfg.into();
        assert_eq!(back.last_workspace, s.last_workspace);
        assert_eq!(back.vim_mode, s.vim_mode);
        assert_eq!(back.theme, s.theme);
        assert_eq!(back.theme_dark, s.theme_dark);
        assert_eq!(back.theme_mode, s.theme_mode);
        assert_eq!(back.font_size, s.font_size);
        assert_eq!(back.sync_transport, s.sync_transport);
        assert_eq!(back.backlinks_order, s.backlinks_order);
        assert_eq!(back.reminders_enabled, s.reminders_enabled);
        assert_eq!(back.reminders_quiet_hours, s.reminders_quiet_hours);
    }

    /// Regression test for the RFC 0022 follow-up that added
    /// `theme_dark` / `theme_mode` to the flat `Settings` DTO.
    ///
    /// Before this change, `preset_dark` / `mode` weren't modeled at
    /// all, so `restore_unmodeled_sections` had to pull both back
    /// from disk after every save — otherwise `From<Settings> for
    /// Config` would reset them to their literal defaults (`None` /
    /// `Auto`) and silently wipe a hand-configured pair. That old
    /// test (`save_keeps_the_theme_pair_but_lets_the_modal_pick_the_preset`)
    /// pinned exactly that restore.
    ///
    /// Now that the modal owns all three `[theme]` fields, restoring
    /// any of them from disk would be the *opposite* bug: it would
    /// silently discard whatever the user just picked for `theme_dark`
    /// / `theme_mode` in the modal, making those controls appear to do
    /// nothing. This test pins the new contract instead: a save must
    /// carry the modal's full theme pick through untouched, while a
    /// section the modal still doesn't model (`[calendar]`, used here
    /// as the representative unmodelled section) is still restored
    /// from disk.
    #[test]
    fn save_round_trips_the_whole_theme_pair_and_still_restores_unmodelled_sections() {
        use outl_config::ThemeMode;

        let mut on_disk = Config::default();
        // Stale on-disk pair the modal is about to override.
        on_disk.theme.preset = "logseq-light".into();
        on_disk.theme.preset_dark = Some("outl".into());
        on_disk.theme.mode = ThemeMode::Light;
        // A hand-set value in a still-unmodelled section, distinct from
        // the default, so a lost restore would show up as a mismatch.
        on_disk.calendar.timezone = Some("America/Sao_Paulo".into());

        let modal_settings = Settings {
            theme: "dracula".into(),
            theme_dark: "nord".into(),
            theme_mode: "dark".into(),
            ..Settings::fresh()
        };
        let mut cfg: Config = modal_settings.into();
        restore_unmodeled_sections(&mut cfg, &on_disk);

        assert_eq!(
            cfg.theme.preset, "dracula",
            "the modal's light pick must win over the stale on-disk value"
        );
        assert_eq!(
            cfg.theme.preset_dark,
            Some("nord".to_string()),
            "the modal's dark pick must win — it is modeled now, so it must NOT be restored from disk"
        );
        assert_eq!(
            cfg.theme.mode,
            ThemeMode::Dark,
            "the modal's mode pick must win — it is modeled now, so it must NOT be restored from disk"
        );
        assert_eq!(
            cfg.calendar, on_disk.calendar,
            "[calendar] is still unmodelled by Settings and must still be restored from disk"
        );
    }
}
