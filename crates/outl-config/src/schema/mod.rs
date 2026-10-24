//! Top-level [`Config`] struct, plus the facade over the sections it is
//! built from.
//!
//! One module per responsibility, not one per type: `ui` is what a client
//! renders, `storage` is the workspace's bytes on this device, `sync` is
//! what moves the op log between those devices, `schedule` is wall-clock
//! time and the windows read against it, `telemetry` is consent to crash
//! reporting. `[tui]` predates the split and still lives in `crate::tui`.
//!
//! Adding a field anywhere is a one-line change in both the struct
//! and (if surfaced to the desktop wire format) the Tauri command
//! shim. `#[serde(default)]` everywhere means a missing field falls
//! back to the type's [`Default`], so an old config file keeps
//! working after the schema grows.

mod schedule;
mod storage;
mod sync;
mod telemetry;
mod ui;

pub use schedule::{CalendarCfg, RemindersCfg};
pub use storage::{AssetsCfg, BackupCfg, SnapshotCfg, StorageCfg, WorkspaceCfg};
pub use sync::{SyncConfig, SyncTransportKind};
pub use telemetry::TelemetryCfg;
pub use ui::{BacklinksOrder, DiagramsMode, DisplayCfg, EditorCfg, ThemeCfg, ThemeMode};

use crate::tui::TuiCfg;
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
    /// Set by an external configuration manager (Nix / home-manager) when
    /// it owns this file. When `true`, no client rewrites `config.toml` —
    /// see [`crate::managed`]. The directive travels inside the file it
    /// describes, so a package manager that generates the file also
    /// declares itself its owner in one write.
    ///
    /// This is a **machine-level** directive, never op-log state (root
    /// `CLAUDE.md` invariant #7): it says how *this* device's file is
    /// produced, not anything about workspace content, so it correctly
    /// lives in the per-device TOML and not the CRDT.
    ///
    /// Must stay the **first** field: `toml::to_string_pretty` serialises
    /// scalar fields before tables, so a top-level key emitted after
    /// `[workspace]` would re-parse as `[workspace].managed`.
    pub managed: bool,
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
    pub telemetry: TelemetryCfg,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let c = Config::default();
        assert_eq!(c.theme.preset, "outl-light");
        assert_eq!(c.theme.dark(), "outl");
        assert!(c.editor.vim_mode);
        assert_eq!(c.editor.font_size, 15);
        assert_eq!(c.editor.diagrams, DiagramsMode::Pele);
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
        assert!(
            c.telemetry.enabled,
            "crash reporting defaults ON — see TelemetryCfg docs"
        );
    }

    #[test]
    fn partial_assets_section_keeps_other_defaults() {
        // A config with ONLY [assets] populated must leave every other
        // section at its default (the schema-change checklist).
        let c: Config = toml::from_str("[assets]\nmax_bytes = 5000\n").unwrap();
        assert_eq!(c.assets.max_bytes, 5000);
        assert_eq!(c.theme.preset, "outl-light");
        assert_eq!(c.theme.dark(), "outl");
        assert!(c.editor.vim_mode);
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
    }
}
