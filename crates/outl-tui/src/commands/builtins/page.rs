//! Page-scoped builtins: property edits and the `pin` toggle.
//!
//! These commands operate on either the current block or the page's
//! top-level properties. They live together because they share the
//! "mutate AST → save → reconcile" code path through `App`.

use anyhow::Result;

use super::super::SlashCommand;
use crate::state::App;

// ---------------------------------------------------------------------------
// prop-block — add or update a property on the current block
// ---------------------------------------------------------------------------

pub struct PropBlockCommand;
impl SlashCommand for PropBlockCommand {
    fn name(&self) -> &'static str {
        "prop-block"
    }
    fn description(&self) -> &'static str {
        "Set a property on the current block — `prop-block <key> <value>` (empty value deletes)"
    }
    fn needs_args(&self) -> bool {
        true
    }
    fn aliases(&self) -> &'static [&'static str] {
        // `prop` defaults to the block scope — that's the common case.
        &["prop"]
    }
    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        let (key, value) = args.split_once(' ').unwrap_or((args, ""));
        let value = value.trim();
        // `/prop oura-date:: x` is how the key is spelled everywhere
        // else, so accept it and store the clean form.
        let key = outl_actions::property::normalize_key(key);
        if let Some(why) = outl_actions::property::key_rejection(&key) {
            app.status = why;
            return Ok(false);
        }
        app.set_property_on_current_block(&key, value);
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// prop-page — add or update a property at page level
// ---------------------------------------------------------------------------

pub struct PropPageCommand;
impl SlashCommand for PropPageCommand {
    fn name(&self) -> &'static str {
        "prop-page"
    }
    fn description(&self) -> &'static str {
        "Set a property on the page itself (`title::`, `icon::`, …) — `prop-page <key> <value>`"
    }
    fn needs_args(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        let (key, value) = args.split_once(' ').unwrap_or((args, ""));
        let value = value.trim();
        let key = outl_actions::property::normalize_key(key);
        if let Some(why) = outl_actions::property::key_rejection(&key) {
            app.status = why;
            return Ok(false);
        }
        app.set_property_on_page(&key, value);
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// pin — toggle the `pinned::` page property
// ---------------------------------------------------------------------------

pub struct PinCommand;
impl SlashCommand for PinCommand {
    fn name(&self) -> &'static str {
        "pin"
    }
    fn description(&self) -> &'static str {
        "Toggle pinned:: on the current page (shows in sidebar Pinned)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        // `pinned` is a synonym — it reads naturally in the palette
        // ("/pinned" → "is this pinned? toggle it"). We intentionally
        // do NOT alias `unpin`: a command named `unpin` that *toggles*
        // would silently pin an unpinned page, which is surprising.
        // Users wanting the explicit verb get `/pin` either way.
        &["pinned"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.toggle_pinned();
        Ok(false)
    }
}
