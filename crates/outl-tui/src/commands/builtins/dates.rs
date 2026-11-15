//! Date / time / week-tag insert commands — the terminal's wiring onto
//! [`outl_actions::insert`].
//!
//! This file used to be nineteen `SlashCommand` structs, each carrying
//! its own name, description, aliases and the string it inserts. That
//! made `outl-tui` the owner of "what does `/date-today` insert", so a
//! desktop user typing it got nothing, and the obvious repair was to
//! write the same nineteen answers again in another language.
//!
//! The catalog moved to `outl-actions`, beside the calendar arithmetic
//! it was already calling. What is left here is the half that is
//! genuinely this client's: writing into a live `EditBuffer`, and
//! refusing politely outside Insert mode.

use anyhow::Result;
use outl_actions::{clock, insert};

use super::super::SlashCommand;
use crate::state::{App, Mode};

/// One catalog entry, wired to this client's Insert buffer.
///
/// Holds a `&'static` entry rather than a name, so a command that is
/// registered cannot fail to resolve — the lookup happened once, at
/// registration.
pub struct InsertTextCommand(&'static insert::InsertCommand);

impl SlashCommand for InsertTextCommand {
    fn name(&self) -> &'static str {
        self.0.name
    }

    fn description(&self) -> &'static str {
        self.0.description
    }

    fn aliases(&self) -> &'static [&'static str] {
        self.0.aliases
    }

    fn needs_args(&self) -> bool {
        self.0.needs_args()
    }

    /// The slash dispatcher reads this to skip `commit_insert()` — the
    /// command needs the buffer alive at the cursor.
    fn inserts_inline(&self) -> bool {
        true
    }

    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        match insert::resolve(self.0, args, clock::now_local()) {
            Ok(text) => {
                // Only Insert mode has a cursor to splice into. Outside
                // it the command says what it *would* have written, so
                // the user can see the value even when they cannot
                // place it.
                if let Mode::Insert { buffer, .. } = &mut app.mode {
                    buffer.insert_str(&text);
                } else {
                    let name = self.0.name;
                    app.status = format!("/{name} only works in Insert mode (would insert {text})");
                }
            }
            // `/date` with an unreadable argument. The usage line is
            // the catalog's, so the terminal and a GUI refuse in the
            // same words.
            Err(e) => app.status = e.to_string(),
        }
        Ok(false)
    }
}

/// Every text-inserting command in the catalog, ready to register.
pub fn all() -> impl Iterator<Item = InsertTextCommand> {
    insert::ALL.iter().map(InsertTextCommand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandRegistry;

    /// A rename in the catalog reaches the terminal without anyone
    /// editing this file, which is the whole point of the move.
    #[test]
    fn every_catalog_entry_is_registered_under_its_own_name() {
        let registry = CommandRegistry::with_builtins();
        for command in insert::ALL {
            let found = registry
                .get(command.name)
                .unwrap_or_else(|| panic!("/{} is not registered", command.name));
            assert_eq!(found.name(), command.name);
            assert!(
                found.inserts_inline(),
                "/{} must keep the Insert buffer alive",
                command.name
            );
        }
    }

    /// The TUI's `:` palette has always accepted `:dt` for
    /// `:date-today`. The catalog carries the aliases, and this is
    /// what proves the registry still reaches them.
    #[test]
    fn an_alias_still_resolves_to_its_command() {
        let registry = CommandRegistry::with_builtins();
        for (alias, name) in [
            ("dt", "date-today"),
            ("now", "time-now"),
            ("wn", "week-num"),
        ] {
            let found = registry
                .get(alias)
                .unwrap_or_else(|| panic!("/{alias} resolves to nothing"));
            assert_eq!(found.name(), name);
        }
    }
}
