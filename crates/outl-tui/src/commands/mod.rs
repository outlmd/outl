//! Slash / palette command system.
//!
//! Both the slash menu (`/`) and the vim command palette (`:`) hit
//! the **same** registry. Plugins will eventually register through
//! `CommandRegistry::register` and immediately show up in both
//! surfaces — there's no second list to keep in sync.
//!
//! The shape mirrors [`outl_exec::RuntimeRegistry`]:
//!
//! - `SlashCommand` is the trait every command implements.
//! - `CommandRegistry` is the lookup; built-ins go in via
//!   `CommandRegistry::with_builtins`.
//! - [`builtins`] holds the shipped commands. Each one is its own
//!   small struct under ~50 lines.

pub mod builtins;

use std::sync::Arc;

use anyhow::Result;

use crate::state::App;

/// One executable command.
///
/// Implementations are stateless — the registry holds them as
/// `Arc<dyn SlashCommand>` for cheap clone-and-share. Args, when the
/// command takes them, arrive as a trimmed `&str` ("`priority high`",
/// not "`prop priority high`"); parsing is the command's job.
pub(crate) trait SlashCommand: Send + Sync {
    /// Name as the user types it (`prop`, `search`, ...).
    fn name(&self) -> &'static str;

    /// One-liner shown in the slash menu next to the name.
    fn description(&self) -> &'static str;

    /// Whether `execute` expects a non-empty `args`. The slash menu
    /// uses this to decide between dispatching directly vs handing
    /// off to the vim command palette pre-filled for arg entry.
    fn needs_args(&self) -> bool {
        false
    }

    /// Optional aliases (e.g. `q` for `quit`, `x` for `run`). Looked
    /// up by the vim palette so legacy short forms keep working.
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether the command writes text directly into the in-flight
    /// Insert buffer (`buffer.insert_str(...)`). When `true`, the
    /// slash dispatcher will *not* commit the Insert before
    /// dispatching — the command needs the buffer alive at the
    /// cursor position. Commands like `/date-today` set this.
    fn inserts_inline(&self) -> bool {
        false
    }

    /// Run the command. May mutate `app`; returns `Ok(true)` if the
    /// app should quit (used by `:q`).
    fn execute(&self, app: &mut App, args: &str) -> Result<bool>;
}

/// Owned collection of [`SlashCommand`]s, keyed by `name` + aliases.
///
/// `pub(crate)` for now — the plugin system will widen this to `pub`
/// (along with [`App`]) when external crates can register commands.
#[derive(Clone, Default)]
pub(crate) struct CommandRegistry {
    commands: Vec<Arc<dyn SlashCommand>>,
}

impl CommandRegistry {
    /// Empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Registry pre-populated with every shipped built-in command.
    pub(crate) fn with_builtins() -> Self {
        let mut r = Self::new();
        builtins::register_all(&mut r);
        r
    }

    /// Add (or replace) a command.
    ///
    /// "Replace" happens by `name` match; useful for plugin overrides.
    /// Aliases collide silently — first registration wins.
    pub(crate) fn register<C: SlashCommand + 'static>(&mut self, cmd: C) -> &mut Self {
        // Remove any existing command with the same primary name so a
        // plugin override is idempotent.
        let name = cmd.name();
        self.commands.retain(|c| c.name() != name);
        self.commands.push(Arc::new(cmd));
        self
    }

    /// Look up by `name` or any alias. Case-sensitive — `prop` and
    /// `Prop` are different (commands are lowercase by convention).
    pub(crate) fn get(&self, name: &str) -> Option<Arc<dyn SlashCommand>> {
        self.commands
            .iter()
            .find(|c| c.name() == name || c.aliases().contains(&name))
            .cloned()
    }

    /// Every command in the registry, in registration order.
    /// The slash menu calls this to populate its list.
    pub(crate) fn all(&self) -> impl Iterator<Item = &Arc<dyn SlashCommand>> {
        self.commands.iter()
    }

    /// Dispatch a full `<name> <args>` line through the registry.
    /// Used by the vim palette and `accept_slash` for arg-less
    /// commands. Returns `Ok(true)` when the command quits the app.
    pub(crate) fn dispatch(&self, app: &mut App, line: &str) -> Result<bool> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(false);
        }
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        match self.get(name) {
            Some(cmd) => cmd.execute(app, rest.trim()),
            None => {
                app.status = format!("unknown command: {name}");
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod catalog_name_tests {
    use super::CommandRegistry;
    use outl_shortcuts::{command_name, Action};

    /// The built-ins that are a second door onto a catalog
    /// [`Action`]. Each pair is a name a user can type in *both*
    /// clients, so the two must resolve it to the same verb.
    ///
    /// Declared rather than derived: a `SlashCommand` has no
    /// `Action`, on purpose — most built-ins (`date-today`,
    /// `theme`, `sync`) have no chord and never will. This list is
    /// the overlap, and it is what fails when one side renames.
    const SHARED_WITH_THE_CATALOG: &[(&str, Action)] = &[
        ("today", Action::OpenToday),
        ("help", Action::ToggleHelp),
        ("quit", Action::Quit),
        ("open", Action::OpenPicker),
        ("pin", Action::TogglePin),
        ("run", Action::RunCodeBlock),
        ("refer", Action::CopyBlockRef),
    ];

    /// A user who learned `/today` here types it on the desktop and
    /// reaches the same thing. Before `command_name` existed the
    /// desktop called it `open-today`, because it kebab-cased the
    /// variant while this crate carried its own literal.
    #[test]
    fn a_shared_builtin_is_named_by_the_catalog() {
        let registry = CommandRegistry::with_builtins();
        for (name, action) in SHARED_WITH_THE_CATALOG {
            assert_eq!(
                command_name(*action),
                *name,
                "the catalog renamed {action:?}; this built-in still answers to `{name}`"
            );
            assert!(
                registry.get(name).is_some(),
                "`/{name}` is no longer registered, so the catalog's name reaches nothing here"
            );
        }
    }

    /// The other direction: a built-in that is *not* in the list
    /// above must not squat a catalog name, or `/run` would mean one
    /// thing in the TUI and another on the desktop.
    #[test]
    fn no_unlisted_builtin_squats_a_catalog_name() {
        let registry = CommandRegistry::with_builtins();
        for cmd in registry.all() {
            let Some(action) = Action::ALL.iter().find(|a| command_name(**a) == cmd.name()) else {
                continue;
            };
            assert!(
                SHARED_WITH_THE_CATALOG.contains(&(cmd.name(), *action)),
                "`/{}` collides with {action:?} in the catalog but is not declared as the same verb",
                cmd.name()
            );
        }
    }
}
