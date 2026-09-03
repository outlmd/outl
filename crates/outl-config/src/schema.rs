//! Top-level [`Config`] struct + its sub-sections.
//!
//! Adding a field anywhere is a one-line change in both the struct
//! and (if surfaced to the desktop wire format) the Tauri command
//! shim. `#[serde(default)]` everywhere means a missing field falls
//! back to the type's [`Default`], so an old config file keeps
//! working after the schema grows.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Root config — three sections that map cleanly to "which client
/// cares".
///
/// - [`WorkspaceCfg`] — read by the desktop (last opened path) and
///   the TUI (when no `--path` flag is passed).
/// - [`ThemeCfg`] — read by every renderer (TUI, desktop) for which
///   `outl_theme::Palette` to render with.
/// - [`EditorCfg`] — local editing preferences, mostly desktop
///   today (the TUI is vim-style by definition).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub workspace: WorkspaceCfg,
    pub theme: ThemeCfg,
    pub editor: EditorCfg,
    pub calendar: CalendarCfg,
    pub sync: SyncConfig,
    pub tui: TuiCfg,
    pub snapshot: SnapshotCfg,
    pub storage: StorageCfg,
    pub display: DisplayCfg,
    pub assets: AssetsCfg,
    pub reminders: RemindersCfg,
    pub backup: BackupCfg,
}

/// Reminder delivery preferences.
///
/// **Device-local on purpose.** Quiet hours are a property of *this*
/// phone / *this* laptop, not of the workspace — the rule itself
/// (`remind:: 3pm every 1h`) lives in the markdown and converges
/// through the op log, and so does a snooze
/// (`outl_core::op::Op::SnoozeRemind`). Putting quiet hours in the
/// op log would silence a laptop because a phone was asleep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemindersCfg {
    /// Master switch, **on by default**.
    ///
    /// Writing `remind:: 3pm` on a block is already the explicit
    /// opt-in — a bare `[[date]]` schedules nothing. Defaulting this
    /// to `false` meant the user wrote the rule, waited, got nothing,
    /// and had to go find a toggle: the feature silently broken out of
    /// the box. A device that never gets a `remind::` never fires, so
    /// the default costs someone who doesn't use reminders nothing,
    /// not even the OS permission prompt (which only appears on the
    /// first actual notification).
    ///
    /// Set it to `false` to keep the rules tracked and listed on this
    /// device while never being interrupted by them.
    pub enabled: bool,

    /// `"22:00-07:00"` — a fire that would land inside this window is
    /// pushed to the window's end instead of dropped. `None` (the
    /// default) means no quiet hours. A window that wraps midnight is
    /// the normal case and is handled.
    ///
    /// An unparseable value is ignored (treated as `None`) rather than
    /// failing the whole config load — a typo here must never keep the
    /// app from opening.
    pub quiet_hours: Option<String>,
}

impl Default for RemindersCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_hours: None,
        }
    }
}

impl RemindersCfg {
    /// Parse [`Self::quiet_hours`] into `(start_minutes, end_minutes)`
    /// past midnight, or `None` when unset / unparseable.
    ///
    /// `start == end` is rejected: a zero-width window is a typo, and
    /// reading it as "quiet all day" would silence every reminder the
    /// user asked for.
    pub fn quiet_window(&self) -> Option<(u32, u32)> {
        let raw = self.quiet_hours.as_deref()?.trim();
        let (start, end) = raw.split_once('-')?;
        let start = parse_hhmm(start.trim())?;
        let end = parse_hhmm(end.trim())?;
        (start != end).then_some((start, end))
    }
}

/// `"22:00"` -> minutes past midnight.
fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

/// Direction of the backlinks ("Linked from") list.
///
/// `lowercase` serde so the TOML reads `backlinks_order = "newest"` /
/// `"oldest"` — how a user thinks of it, not the Rust variant casing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BacklinksOrder {
    /// Most recently referenced page first (the product default).
    #[default]
    Newest,
    /// Oldest referenced page first.
    Oldest,
}

impl BacklinksOrder {
    /// Whether the newest reference sorts to the top — the `bool` the
    /// `outl_actions::sort_backlinks` renderer path expects.
    pub fn newest_first(self) -> bool {
        matches!(self, BacklinksOrder::Newest)
    }
}

/// Display section — cross-client presentation preferences that are
/// pure view state (they never converge between devices, same policy
/// as [`ThemeCfg`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayCfg {
    /// Direction of the backlinks list. Read by every renderer (TUI at
    /// boot, the GUI clients when building a `PageView`). Default
    /// [`BacklinksOrder::Newest`] — the fix for issue #142, where long
    /// backlink lists buried the latest reference at the bottom.
    pub backlinks_order: BacklinksOrder,
}

/// TUI-only preferences (the desktop ignores this section).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiCfg {
    /// Capture the mouse so the app owns selection: drag across blocks
    /// selects a range and copies it as clean markdown on release, the
    /// scroll wheel moves the selection, a click selects a block.
    ///
    /// Default `false`, and deliberately opt-in: capturing the mouse
    /// **disables the terminal's own text selection** (selecting a URL,
    /// copying a single word, dragging across panes), which is muscle
    /// memory for many terminal users. Turn it on only if you want
    /// mouse-driven copy inside outl more than the terminal's native
    /// selection. The keyboard yank (`yy` / `Y` / Visual `y`) copies
    /// markdown to the clipboard regardless of this flag.
    pub mouse_capture: bool,
}

/// Workspace section — primarily where the desktop remembers the
/// last opened directory so the next launch skips the picker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceCfg {
    /// Absolute path to the last workspace the user opened. The
    /// desktop writes this on every `set_workspace` call; the TUI
    /// can read it as a fallback when no `--path` flag was given.
    pub last: Option<PathBuf>,
}

/// Which side of the light/dark preset pair to render.
///
/// Names a *side*, not a colour: nothing stops a user putting a dark
/// preset in `preset`, and then `Light` returns it. That is a
/// misconfigured pair rather than a resolution bug — `outl doctor`
/// reports it (RFC 0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Always `preset`.
    Light,
    /// Always `ThemeCfg::dark()`.
    Dark,
    /// Follow the OS appearance. The TUI cannot read it and treats
    /// this as `Dark` — see `docs/theming.md` → "Light / dark pair
    /// and `mode`".
    #[default]
    Auto,
}

/// Theme section. Preset names match `outl_theme::PRESETS`
/// (`"outl"`, `"dracula"`, …); unknown names fall back to
/// `outl_theme::default()` at render time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeCfg {
    /// The light side of the pair, and the only preset used when
    /// `mode` is `Light`.
    pub preset: String,
    /// The dark side. `None` resolves to [`ThemeCfg::preset`] — that
    /// fallback is what keeps every pre-RFC-0022 config (which has
    /// only `preset`) behaving exactly as it did.
    pub preset_dark: Option<String>,
    pub mode: ThemeMode,
}

impl ThemeCfg {
    /// The dark-side preset name, falling back to [`Self::preset`]
    /// when the user never configured a pair.
    ///
    /// Do not inline this as `preset_dark.unwrap_or_default()` — an
    /// empty string is not a preset name and would silently resolve
    /// to `outl_theme::default()`, changing the theme of every
    /// existing config.
    pub fn dark(&self) -> &str {
        self.preset_dark.as_deref().unwrap_or(&self.preset)
    }
}

impl Default for ThemeCfg {
    fn default() -> Self {
        Self {
            preset: "outl".to_string(),
            preset_dark: None,
            mode: ThemeMode::Auto,
        }
    }
}

/// Editor preferences. `vim_mode` defaults to `true` because
/// outl is keyboard-first — the same default the TUI ships with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorCfg {
    /// Vim-style modal bindings (Normal / Insert / Visual).
    /// When `false`, the desktop falls back to plain text-editing
    /// chrome (no modes; OS-standard chords only). The TUI is
    /// vim-style by definition and ignores this flag.
    pub vim_mode: bool,

    /// Base font size for the outline view (pixels). The TUI
    /// doesn't read this; terminal font is the user's terminal
    /// setting.
    pub font_size: u32,
}

impl Default for EditorCfg {
    fn default() -> Self {
        Self {
            vim_mode: true,
            font_size: 15,
        }
    }
}

/// Calendar / time section — controls how outl renders "now" and
/// "today".
///
/// `timezone` is an optional IANA name (`"Europe/London"`,
/// `"America/Sao_Paulo"`). When unset, outl uses the operating
/// system's local timezone — the right default on a normally
/// configured machine. Set it explicitly when the OS clock lies about
/// the zone: containers and Chrome OS **Crostini** run in UTC even
/// though the user's real timezone isn't, which pushes the journal
/// date and the status-line clock an hour (or more) off. See issue
/// #107.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarCfg {
    /// IANA timezone name, e.g. `"Europe/London"`. `None` (the
    /// default) means "use the OS local timezone". An unknown or
    /// unparseable name is ignored when the clock initializes (logged)
    /// and also falls back to local.
    pub timezone: Option<String>,
}

/// Which sync transport a client wires up at boot.
///
/// `lowercase` serde so the TOML reads `transport = "file"` /
/// `transport = "iroh"` — matching how a user thinks of them, not
/// the Rust variant casing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncTransportKind {
    /// File-based transport (iCloud Drive / shared filesystem). The opt-out
    /// from iroh: set `transport = "file"`. Still fully supported.
    File,
    /// iroh P2P transport (QUIC + hole punching). The default — P2P is
    /// outl's primary sync. Override with `transport = "file"`.
    #[default]
    Iroh,
}

/// Sync section. Controls which transport moves the per-actor op log
/// between devices. Missing `[sync]` falls back to [`SyncTransportKind::Iroh`]
/// (P2P is outl's primary sync); `transport = "file"` is the explicit opt-out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Transport to use. Defaults to [`SyncTransportKind::Iroh`].
    pub transport: SyncTransportKind,

    /// Optional relay URL for the `iroh` transport. `None` (or an
    /// empty string in the TOML, normalized to `None` on read) means
    /// use outl's default relay (`use1-1.relay.avelino.outl.iroh.link`). Ignored by the
    /// `file` transport.
    pub relay_url: Option<String>,
}

impl SyncConfig {
    /// The configured relay URL, with empty strings treated as
    /// "unset". A user who writes `relay_url = ""` in TOML to mean
    /// "use the defaults" gets `None`, same as omitting the key.
    pub fn relay_url(&self) -> Option<&str> {
        self.relay_url.as_deref().filter(|s| !s.is_empty())
    }
}

/// Snapshot section — controls when long-lived clients persist a
/// materialized-state snapshot of the workspace to disk.
///
/// Snapshots are a boot cache: `Workspace::open_with_storage` loads
/// one (if present and its `content_hash` matches) and replays only
/// the ops posted after its cutoff, instead of walking the entire
/// per-actor op log. This is the fix for issue #109 (CLI pays full
/// replay on every invocation).
///
/// The CLI never writes snapshots — it's ephemeral — but reads any
/// produced by a long-lived client (TUI / desktop / mobile) so it
/// still gets the speedup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotCfg {
    /// Master switch. `false` makes `Workspace::apply` skip the
    /// snapshot-trigger check entirely. The CLI sets this to `false`
    /// in-memory regardless of the TOML.
    pub enabled: bool,

    /// How many ops a long-lived client applies between snapshot
    /// writes. Lower values shrink the post-snapshot delta (faster
    /// boot) at the cost of more writes; higher values write less
    /// often but leave more ops to replay. The default of `10_000`
    /// is roughly "once per long editing session" for an individual
    /// user — rare enough not to stutter, frequent enough that the
    /// boot delta stays in the hundreds of milliseconds.
    pub op_threshold: u32,
}

impl Default for SnapshotCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            op_threshold: 10_000,
        }
    }
}

/// Storage section — controls `JsonlStorage`'s in-memory footprint
/// (RFC #137). The op-log cache is a bounded LRU; ops evicted from
/// RAM are addressable through the per-actor offset index. This keeps
/// RSS roughly constant regardless of how much history the workspace
/// has accumulated.
///
/// Defaults are conservative: 20k ops ≈ 4 MB of cache on the desktop,
/// enough for the home page plus its backlinks; mobile pins to 5k
/// (≈ 1 MB) at boot to stay well under iOS jetsam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageCfg {
    /// Maximum number of ops held in RAM per `JsonlStorage` instance.
    /// `0` is treated as "unbounded" (legacy behaviour — every op
    /// stays resident). Anything `> 0` enforces the LRU cap.
    pub lru_cap: usize,
}

impl Default for StorageCfg {
    fn default() -> Self {
        Self { lru_cap: 20_000 }
    }
}

/// Assets section — policy for uploaded files (`assets/<hash>.<ext>`).
///
/// The directory itself is fixed at `<workspace>/assets/` (part of the
/// workspace layout, not configurable), so the only policy here is the
/// upper bound on a single upload. A file over the cap is rejected
/// before it is copied, so a fat-fingered drag of a multi-GB file can't
/// balloon the workspace (and, on the P2P transport, every paired
/// device).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetsCfg {
    /// Maximum size, in bytes, of a single uploaded file. `0` means
    /// "unbounded". Default is 100 MiB.
    pub max_bytes: u64,
}

impl Default for AssetsCfg {
    fn default() -> Self {
        Self {
            max_bytes: 100 * 1024 * 1024,
        }
    }
}

/// Automatic local backups of the workspace (`outl_actions::backup`).
///
/// **Device-local, like every other preference here.** A backup is a
/// property of *this* machine's disk, not of the workspace: two paired
/// devices each keep their own history, and neither one's git repo
/// travels through the op log or the sync transport — the repository
/// itself lives outside the workspace, under `outl_core::device_dir()`,
/// precisely so no file transport can replicate it.
///
/// Read today by the **TUI** only (`outl_actions::backup::spawn_auto_pass`
/// at startup); the desktop and mobile clients preserve the section
/// verbatim but do not yet run the pass. See
/// [`docs/config.md`](https://github.com/outlmd/outl/blob/main/docs/config.md)
/// → `[backup]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupCfg {
    /// Take automatic snapshots. **On by default.**
    ///
    /// The failure this guards against — a projection bug, a mis-aimed
    /// import, a page deleted and the app closed before undo could help
    /// — is exactly the kind a user only discovers later, when the
    /// window to enable a safety net has already closed. Defaulting off
    /// means the feature is present for everyone who didn't need it and
    /// absent for everyone who did.
    ///
    /// Costs nothing on a workspace that never changes (an unchanged
    /// tree produces no commit) and degrades to a `warn!` when there is
    /// no `git` on `PATH`.
    pub enabled: bool,

    /// Minimum minutes between automatic snapshots. Default 30.
    ///
    /// A floor, not a schedule: the background pass wakes on this
    /// cadence and takes a snapshot only when at least this much time
    /// has passed since the newest commit, so a burst of edits never
    /// turns into a burst of commits. The elapsed time is read back out
    /// of the git history, not from a state file, so it survives a
    /// restart.
    pub interval_minutes: u64,
}

impl Default for BackupCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let c = Config::default();
        assert_eq!(c.theme.preset, "outl");
        assert!(c.editor.vim_mode);
        assert_eq!(c.editor.font_size, 15);
        assert!(c.workspace.last.is_none());
        assert!(c.calendar.timezone.is_none());
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert!(c.sync.relay_url.is_none());
        assert!(c.snapshot.enabled);
        assert_eq!(c.snapshot.op_threshold, 10_000);
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
        assert!(c.display.backlinks_order.newest_first());
        assert!(c.backup.enabled, "backups default ON — see BackupCfg docs");
        assert_eq!(c.backup.interval_minutes, 30);
    }

    /// The partial-TOML path required when adding a section: a config
    /// that predates `[backup]` must still get the defaults.
    #[test]
    fn backup_section_defaults_when_absent() {
        let c: Config = toml::from_str("[theme]\npreset = \"outl\"\n").unwrap();
        assert!(c.backup.enabled);
        assert_eq!(c.backup.interval_minutes, 30);
    }

    #[test]
    fn backup_section_parses() {
        let c: Config =
            toml::from_str("[backup]\nenabled = false\ninterval_minutes = 120\n").unwrap();
        assert!(!c.backup.enabled);
        assert_eq!(c.backup.interval_minutes, 120);
    }

    #[test]
    fn display_section_parses_backlinks_order() {
        let c: Config = toml::from_str("[display]\nbacklinks_order = \"oldest\"\n").unwrap();
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Oldest);
        assert!(!c.display.backlinks_order.newest_first());
    }

    #[test]
    fn missing_display_section_defaults_to_newest() {
        // Only [theme] populated → display falls back to its default
        // (newest-first), so an older config keeps the issue-#142 fix.
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
    }

    #[test]
    fn partial_assets_section_keeps_other_defaults() {
        // A config with ONLY [assets] populated must leave every other
        // section at its default (the schema-change checklist).
        let c: Config = toml::from_str("[assets]\nmax_bytes = 5000\n").unwrap();
        assert_eq!(c.assets.max_bytes, 5000);
        assert_eq!(c.theme.preset, "outl");
        assert!(c.editor.vim_mode);
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
    }

    #[test]
    fn missing_assets_section_defaults_to_100_mib() {
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert_eq!(c.assets.max_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn calendar_section_parses_timezone() {
        let c: Config = toml::from_str("[calendar]\ntimezone = \"Europe/London\"\n").unwrap();
        assert_eq!(c.calendar.timezone.as_deref(), Some("Europe/London"));
    }

    #[test]
    fn missing_calendar_section_leaves_timezone_unset() {
        // No [calendar] → timezone None → clock uses OS local (previous behaviour).
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert!(c.calendar.timezone.is_none());
    }

    #[test]
    fn empty_config_defaults_to_iroh_transport() {
        // P2P is outl's primary sync, so a missing [sync] section defaults to
        // iroh. `transport = "file"` is the explicit opt-out.
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_section_parses_file_transport() {
        let c: Config = toml::from_str("[sync]\ntransport = \"file\"\n").unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::File);
    }

    #[test]
    fn sync_section_parses_iroh_transport() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
"#,
        )
        .unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        // No relay set → falls back to defaults (None).
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_empty_relay_url_normalizes_to_none() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
relay_url = ""
"#,
        )
        .unwrap();
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_relay_url_is_returned_when_set() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
relay_url = "https://relay.example"
"#,
        )
        .unwrap();
        assert_eq!(c.sync.relay_url(), Some("https://relay.example"));
    }

    #[test]
    fn missing_snapshot_section_uses_defaults() {
        // No [snapshot] → enabled=true, op_threshold=10_000 (the
        // long-lived clients' default). A user who never heard of
        // snapshots still gets the boot speedup.
        let c: Config = toml::from_str("").unwrap();
        assert!(c.snapshot.enabled);
        assert_eq!(c.snapshot.op_threshold, 10_000);
    }

    #[test]
    fn snapshot_section_can_disable_writes() {
        // The CLI overrides this in-memory regardless, but a user can
        // also opt out globally by writing `enabled = false`.
        let c: Config = toml::from_str("[snapshot]\nenabled = false\n").unwrap();
        assert!(!c.snapshot.enabled);
        // op_threshold keeps its default.
        assert_eq!(c.snapshot.op_threshold, 10_000);
    }

    #[test]
    fn snapshot_section_can_tune_threshold() {
        let c: Config = toml::from_str("[snapshot]\nop_threshold = 1000\n").unwrap();
        assert_eq!(c.snapshot.op_threshold, 1000);
        // enabled keeps its default.
        assert!(c.snapshot.enabled);
    }

    #[test]
    fn reminders_deliver_by_default() {
        // `remind::` on a block IS the opt-in. A device that never
        // gets a rule never fires, so defaulting off only bought the
        // user a rule that silently did nothing.
        let c: Config = toml::from_str("").unwrap();
        assert!(c.reminders.enabled);
        assert_eq!(c.reminders.quiet_window(), None);
    }

    #[test]
    fn reminders_can_still_be_switched_off() {
        let c: Config = toml::from_str("[reminders]\nenabled = false\n").unwrap();
        assert!(!c.reminders.enabled);
    }

    #[test]
    fn quiet_hours_parses_a_wrapping_window() {
        let c: Config =
            toml::from_str("[reminders]\nenabled = true\nquiet_hours = \"22:00-07:00\"\n").unwrap();
        assert!(c.reminders.enabled);
        assert_eq!(c.reminders.quiet_window(), Some((22 * 60, 7 * 60)));
    }

    #[test]
    fn unparseable_quiet_hours_is_ignored_not_fatal() {
        // A typo here must never keep the app from opening.
        for bad in ["22:00", "banana", "25:00-07:00", "22:00-22:00", ""] {
            let c: Config =
                toml::from_str(&format!("[reminders]\nquiet_hours = \"{bad}\"\n")).unwrap();
            assert_eq!(c.reminders.quiet_window(), None, "{bad} should not parse");
        }
    }

    #[test]
    fn a_config_with_only_preset_behaves_exactly_as_before() {
        // RFC 0022's whole backwards-compatibility story. `mode`
        // defaults to auto, but with no `preset_dark` both sides of
        // the pair are the same preset, so auto alternates between
        // dracula and dracula — today's behaviour, byte for byte.
        // If someone "simplifies" the None fallback away, every
        // pre-RFC config silently starts theme-switching.
        let c: Config = toml::from_str("[theme]\npreset = \"dracula\"\n").unwrap();
        assert_eq!(c.theme.preset, "dracula");
        assert_eq!(c.theme.dark(), "dracula");
        assert_eq!(c.theme.mode, ThemeMode::Auto);
    }

    #[test]
    fn a_pair_keeps_the_two_sides_apart() {
        let c: Config = toml::from_str(
            "[theme]\nmode = \"auto\"\npreset = \"logseq-light\"\npreset_dark = \"outl\"\n",
        )
        .unwrap();
        assert_eq!(c.theme.preset, "logseq-light");
        assert_eq!(c.theme.dark(), "outl");
        assert_eq!(c.theme.mode, ThemeMode::Auto);
    }

    #[test]
    fn mode_round_trips_through_toml() {
        for (text, want) in [
            ("light", ThemeMode::Light),
            ("dark", ThemeMode::Dark),
            ("auto", ThemeMode::Auto),
        ] {
            let c: Config = toml::from_str(&format!("[theme]\nmode = \"{text}\"\n")).unwrap();
            assert_eq!(c.theme.mode, want);
            let back = toml::to_string(&c).unwrap();
            assert!(back.contains(&format!("mode = \"{text}\"")), "{back}");
        }
    }
}
