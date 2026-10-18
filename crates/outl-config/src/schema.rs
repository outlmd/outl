//! Top-level [`Config`] struct + its sub-sections.
//!
//! Adding a field anywhere is a one-line change in both the struct
//! and (if surfaced to the desktop wire format) the Tauri command
//! shim. `#[serde(default)]` everywhere means a missing field falls
//! back to the type's [`Default`], so an old config file keeps
//! working after the schema grows.

use std::path::PathBuf;

use crate::tui::TuiCfg;
use serde::{Deserialize, Deserializer, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Deserialize)]
struct ThemeCfgWire {
    preset: Option<String>,
    preset_dark: Option<String>,
    mode: Option<ThemeMode>,
}

impl<'de> Deserialize<'de> for ThemeCfg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ThemeCfgWire::deserialize(deserializer)?;
        let defaults = Self::default();
        let preset_was_explicit = wire.preset.is_some();
        Ok(Self {
            preset: wire.preset.unwrap_or(defaults.preset),
            // A present section with an explicit legacy `preset` but no
            // `preset_dark` must keep one-preset behaviour. A present empty
            // section is equivalent to a fresh config and gets the brand pair.
            preset_dark: if preset_was_explicit {
                wire.preset_dark
            } else {
                wire.preset_dark.or(defaults.preset_dark)
            },
            mode: wire.mode.unwrap_or(defaults.mode),
        })
    }
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
            preset: "outl-light".to_string(),
            preset_dark: Some("outl".to_string()),
            mode: ThemeMode::Auto,
        }
    }
}

/// How a GUI client paints a mermaid fence.
///
/// Device-local, like the rest of `[editor]`. Two devices may
/// disagree and that is fine: the fence on disk does not change.
/// The TUI ignores this and keeps the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiagramsMode {
    /// Render a mermaid fence with Pele.
    #[default]
    Pele,
    /// Highlight the fence as source.
    Off,
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

    /// Mermaid fences in the desktop and mobile apps.
    /// `"pele"` (default) draws them. `"off"` leaves the source.
    pub diagrams: DiagramsMode,
}

impl Default for EditorCfg {
    fn default() -> Self {
        Self {
            vim_mode: true,
            font_size: 15,
            diagrams: DiagramsMode::Pele,
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
mod tests;
