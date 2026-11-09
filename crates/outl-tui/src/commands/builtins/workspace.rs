//! Workspace / chrome commands: navigation, save, reload, help, quit.
//!
//! Anything that doesn't insert text and doesn't touch a single
//! block's properties — these are the "I want to do something with
//! the *editor*" verbs.

use anyhow::Result;

use super::super::SlashCommand;
use crate::state::{App, ToastKind};

// ---------------------------------------------------------------------------
// today — jump to today's journal
// ---------------------------------------------------------------------------

pub struct TodayCommand;
impl SlashCommand for TodayCommand {
    fn name(&self) -> &'static str {
        "today"
    }
    fn description(&self) -> &'static str {
        "Jump to today's journal"
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.go_today()?;
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// refresh — re-read the workspace from disk
// ---------------------------------------------------------------------------

pub struct RefreshCommand;
impl SlashCommand for RefreshCommand {
    fn name(&self) -> &'static str {
        "refresh"
    }
    fn description(&self) -> &'static str {
        "Re-read the workspace from disk (rebuilds index)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["reload", "r"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.refresh_workspace();
        app.toast(ToastKind::Info, "refreshed");
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// write — force-save the current page
// ---------------------------------------------------------------------------

pub struct WriteCommand;
impl SlashCommand for WriteCommand {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Save the current page to disk"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["w", "save"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.save();
        app.toast(ToastKind::Success, "saved");
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// help — toggle the help popup
// ---------------------------------------------------------------------------

pub struct HelpCommand;
impl SlashCommand for HelpCommand {
    fn name(&self) -> &'static str {
        "help"
    }
    fn description(&self) -> &'static str {
        "Toggle the help popup"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["h"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        // Genuine toggle so /help on an open popup closes it — matches
        // the description and matches the `?` keybinding's behavior.
        app.show_help = !app.show_help;
        if !app.show_help {
            app.help_scroll = 0;
        }
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// quit — close the TUI
// ---------------------------------------------------------------------------

pub struct QuitCommand;
impl SlashCommand for QuitCommand {
    fn name(&self) -> &'static str {
        "quit"
    }
    fn description(&self) -> &'static str {
        "Close the TUI (commits any pending Insert first)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["q", "exit"]
    }
    fn execute(&self, _app: &mut App, _args: &str) -> Result<bool> {
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// open — open (or create) a page by title
// ---------------------------------------------------------------------------

pub struct OpenCommand;
impl SlashCommand for OpenCommand {
    fn name(&self) -> &'static str {
        "open"
    }
    fn description(&self) -> &'static str {
        "Open (or create) a page by name — `open <name>`"
    }
    fn needs_args(&self) -> bool {
        true
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["o", "new", "n"]
    }
    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        if args.is_empty() {
            app.status = "usage: open <page name>".into();
            return Ok(false);
        }
        app.open_page_by_name(args)?;
        Ok(false)
    }
}

// ---------------------------------------------------------------------------

/// Open the plugin-settings overlay: edit installed plugins' config and
/// keychain-backed secrets.
pub struct PluginSettingsCommand;

impl SlashCommand for PluginSettingsCommand {
    fn name(&self) -> &'static str {
        "plugin-settings"
    }

    fn description(&self) -> &'static str {
        "Configure installed plugins (config + secrets)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["plugins"]
    }

    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.open_plugin_settings();
        Ok(false)
    }
}

/// `:sync` — per-device pending and the merged edits log, with undo.
pub struct SyncCommand;

impl SlashCommand for SyncCommand {
    fn name(&self) -> &'static str {
        "sync"
    }

    fn description(&self) -> &'static str {
        "What each device is waiting for, and merged edits you can undo"
    }

    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.open_sync_panel();
        Ok(false)
    }
}

/// `/trash` — every block deleted out of this workspace, and the way
/// back (issue #287).
///
/// The command registry rather than a chord: one registration serves
/// both `/trash` (the discoverable slash menu, which lists it with this
/// description) and `:trash`. A chord would need an `Action` variant,
/// and `outl_shortcuts::support`'s exhaustive `match` is frozen at its
/// size baseline — the per-client verdicts it would force are the point
/// of that catalog, not a cost, so this is a deferral with a file
/// behind it. `Capability::Trash` records where the trash is reachable.
pub struct TrashCommand;

impl SlashCommand for TrashCommand {
    fn name(&self) -> &'static str {
        "trash"
    }

    fn description(&self) -> &'static str {
        "What you have deleted, and putting one back"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["deleted"]
    }

    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.open_trash();
        Ok(false)
    }
}
