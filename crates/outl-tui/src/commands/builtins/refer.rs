//! `/refer` and `/refer-embed` — capture the current block's ref/embed
//! handle for later paste.
//!
//! These are normal-mode commands (palette `:refer` or slash `/refer`
//! from outside Insert). They do **not** insert into the buffer — they
//! stash `((handle))` / `!((handle))` in `App::last_yanked_ref` and
//! surface the token on the status line so the user can copy it via
//! the terminal's own selection or via a future clipboard binding.
//!
//! Lookup goes through `App::current_block_ref_handle`, which walks
//! the workspace index for the selected `(slug, dfs_path)`. Identical
//! semantics to the `yr` chord; these commands give the same
//! capability a discoverable, typeable name.

use anyhow::Result;
use outl_shortcuts::{command_name, Action};

use super::super::SlashCommand;
use crate::state::App;

/// `/refer` — capture `((blk-XXXXXX))` for the current block.
pub struct ReferCommand;
impl SlashCommand for ReferCommand {
    fn name(&self) -> &'static str {
        command_name(Action::CopyBlockRef)
    }
    fn description(&self) -> &'static str {
        "Capture ((blk-XXXXXX)) of the current block (stash + status, no insert)"
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.yank_current_ref();
        Ok(false)
    }
}

/// `/refer-embed` — capture `!((blk-XXXXXX))` for the current block.
pub struct ReferEmbedCommand;
impl SlashCommand for ReferEmbedCommand {
    fn name(&self) -> &'static str {
        "refer-embed"
    }
    fn description(&self) -> &'static str {
        "Capture !((blk-XXXXXX)) embed of the current block (stash + status, no insert)"
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.yank_current_embed();
        Ok(false)
    }
}
