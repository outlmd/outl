//! The text-inserting slash commands (`/date-today`, `/time-now`,
//! `/week-num`), over the wire.
//!
//! Pure functions over [`outl_actions::insert`] — no workspace, no
//! lock, same shape as `commands::theme`. The catalog is the single
//! owner of both the names and the strings they produce, so a GUI
//! renders the list and splices the answer rather than holding
//! nineteen date formats of its own.

use outl_actions::{clock, insert};

/// One text-inserting command, for a client's command menu.
#[derive(serde::Serialize)]
pub struct InsertCommandDto {
    /// What the user types after `/`.
    pub name: &'static str,
    /// The one-liner shown beside the name.
    pub description: &'static str,
    /// Short spellings that resolve to the same command.
    pub aliases: &'static [&'static str],
    /// Whether the command needs an argument to answer.
    ///
    /// A client whose `/` trigger closes on the first space cannot
    /// collect one, and should leave those rows out rather than offer
    /// a command that can only fail.
    pub needs_args: bool,
}

/// Every text-inserting command, in menu order.
pub fn list_insert_commands() -> Vec<InsertCommandDto> {
    insert::ALL
        .iter()
        .map(|c| InsertCommandDto {
            name: c.name,
            description: c.description,
            aliases: c.aliases,
            needs_args: c.needs_args(),
        })
        .collect()
}

/// The text `name` inserts right now.
///
/// The clock is read **here** rather than in the client: the instant
/// has to resolve in the configured `[calendar] timezone`, and a
/// webview's `new Date()` is the browser's zone. That is the same
/// mistake `outl_actions::clock` exists to prevent, and the one the
/// reminders DTOs already avoid by crossing the bridge as local
/// strings.
///
/// The error is the catalog's own wording, so a GUI and the terminal
/// refuse an unreadable `/date` argument in the same words.
pub fn resolve_insert_command(name: String, args: Option<String>) -> Result<String, String> {
    insert::resolve_named(&name, args.as_deref().unwrap_or(""), clock::now_local())
        .map_err(|e| e.to_string())
}
