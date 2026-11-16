//! Execution-flavored builtins: search, run a code block, swap theme.
//!
//! These three live together because they each trigger a "side
//! effect that's not an AST edit" — opening an overlay, invoking
//! `outl-exec`, or hot-swapping the palette.

use anyhow::Result;
use outl_shortcuts::{command_name, Action};

use super::super::SlashCommand;
use crate::state::App;
use crate::theme;

// ---------------------------------------------------------------------------
// search — workspace-wide block search
// ---------------------------------------------------------------------------

pub struct SearchCommand;
impl SlashCommand for SearchCommand {
    fn name(&self) -> &'static str {
        "search"
    }
    fn description(&self) -> &'static str {
        "Open the workspace search overlay"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["s", "find"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.open_search();
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// run — execute the code block under the cursor
// ---------------------------------------------------------------------------

pub struct RunCommand;
impl SlashCommand for RunCommand {
    fn name(&self) -> &'static str {
        command_name(Action::RunCodeBlock)
    }
    fn description(&self) -> &'static str {
        "Run the code block under the cursor through outl-exec"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["x", "execute"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.run_current_block();
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// theme — switch palette at runtime
// ---------------------------------------------------------------------------

pub struct ThemeCommand;
impl SlashCommand for ThemeCommand {
    fn name(&self) -> &'static str {
        "theme"
    }
    fn description(&self) -> &'static str {
        "Switch the active theme — `theme <preset>`"
    }
    fn needs_args(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        if args.is_empty() {
            app.status = "usage: theme <preset>".into();
            return Ok(false);
        }
        if let Some(t) = theme::by_name(args) {
            let name = t.name.clone();
            app.theme = t;
            app.status = format!("theme: {name}");
        } else {
            app.status = format!("unknown theme: {args}");
        }
        Ok(false)
    }
}
