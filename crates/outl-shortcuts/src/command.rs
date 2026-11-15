//! The name the user **types** to reach an [`Action`].
//!
//! ## Why this exists
//!
//! [`crate::Action`] names an operation, [`crate::default_bindings`]
//! names the chord that fires it, and [`crate::support`] says which
//! client performs it. None of those answers the fourth question a
//! `/` menu asks: **what does the user type?**
//!
//! That fact had no owner, so it grew two:
//!
//! - the TUI's `SlashCommand::name()`, one `&'static str` per
//!   built-in (`today`, `pin`, `help`, `run`, `refer`, `open`);
//! - the desktop's `actionSlug()`, a kebab-case derivation of the
//!   variant name (`OpenToday` → `open-today`).
//!
//! They can never agree, because they derive from different things.
//! Of 31 TUI built-ins and 66 desktop action rows, exactly **two**
//! names coincided (`quit`, `upload`), and both by accident. A user
//! who learned `/today` in the TUI typed it on the desktop and got a
//! row called `open-today` — close enough to find by substring, far
//! enough to read as a different product.
//!
//! [`command_name`] is the single owner. It is an exhaustive `match`,
//! the same shape [`crate::support`] uses, so a new [`Action`]
//! variant does not compile until its name is declared.
//!
//! ## How a name is chosen
//!
//! **An established TUI name wins.** The TUI shipped first and people
//! have the verbs in their fingers; renaming `/today` to teach them a
//! derivation rule would be the catalog serving itself. Everything
//! else is the kebab-case of the variant, which is what the desktop
//! already showed.
//!
//! Names are unique across the catalog (`no_two_actions_share_a_name`)
//! because a `/` menu dispatches on this string: two actions under one
//! name is a menu row whose behaviour depends on `match` order.

use crate::action::Action;

/// The command name for `action` — what the user types after `/`.
///
/// Stable, lowercase, kebab-case, unique across the catalog. A client
/// renders this in its command menu and dispatches on it; never derive
/// it from the variant name locally, or the clients drift apart again.
pub fn command_name(action: Action) -> &'static str {
    match action {
        // ── chrome ───────────────────────────────────────────────
        // `open` and `help` are the TUI's names (`OpenCommand`,
        // `HelpCommand` in `outl-tui/src/commands/builtins/`).
        Action::OpenPicker => "open",
        Action::OpenCommandPalette => "open-command-palette",
        Action::ToggleHelp => "help",
        Action::ToggleSidebar => "toggle-sidebar",
        Action::ToggleBacklinks => "toggle-backlinks",
        Action::OpenSettings => "open-settings",
        Action::QuickCapture => "quick-capture",
        Action::ExportMarkdown => "export-markdown",
        Action::ToggleFocusMode => "toggle-focus-mode",
        Action::Quit => "quit",

        // ── navigation ───────────────────────────────────────────
        // `today` is the TUI's `TodayCommand`, and the reason this
        // module exists.
        Action::OpenToday => "today",
        Action::OpenYesterday => "yesterday",
        Action::OpenTomorrow => "tomorrow",
        Action::PrevDay => "prev-day",
        Action::NextDay => "next-day",
        Action::SelectionDown => "selection-down",
        Action::SelectionUp => "selection-up",
        Action::OpenRefUnderCursor => "open-ref-under-cursor",

        // ── insert / cursor ──────────────────────────────────────
        Action::EnterInsert => "enter-insert",
        Action::EnterInsertAtStart => "enter-insert-at-start",
        Action::EnterInsertAfter => "enter-insert-after",
        Action::EnterInsertAtEnd => "enter-insert-at-end",
        Action::DeleteCharUnderCursor => "delete-char-under-cursor",
        Action::DeleteCharBeforeCursor => "delete-char-before-cursor",
        Action::DeleteToEndOfBlock => "delete-to-end-of-block",
        Action::ChangeToEndOfBlock => "change-to-end-of-block",
        Action::SubstituteBlock => "substitute-block",
        Action::SubstituteChar => "substitute-char",
        Action::ReplaceChar => "replace-char",
        Action::FindCharForward => "find-char-forward",
        Action::FindCharBackward => "find-char-backward",
        Action::ToggleCharCase => "toggle-char-case",
        Action::CursorWordEnd => "cursor-word-end",

        // ── view ─────────────────────────────────────────────────
        Action::UnfoldAll => "unfold-all",
        Action::FoldAll => "fold-all",
        Action::CenterViewport => "center-viewport",
        Action::ZoomIn => "zoom-in",
        Action::ZoomOut => "zoom-out",
        Action::SearchWordForward => "search-word-forward",
        Action::SearchWordBackward => "search-word-backward",
        Action::ReselectLastVisual => "reselect-last-visual",
        Action::IndentVisualRange => "indent-visual-range",
        Action::OutdentVisualRange => "outdent-visual-range",

        // ── block structure ──────────────────────────────────────
        Action::NewBlockBelow => "new-block-below",
        Action::NewBlockAbove => "new-block-above",
        Action::IndentBlock => "indent-block",
        Action::OutdentBlock => "outdent-block",
        Action::MoveBlockUp => "move-block-up",
        Action::MoveBlockDown => "move-block-down",
        Action::DeleteBlock => "delete-block",
        Action::ToggleCollapsed => "toggle-collapsed",
        Action::ToggleTodo => "toggle-todo",
        Action::MarkDone => "mark-done",
        // `refer` is the TUI's `ReferCommand`.
        Action::CopyBlockRef => "refer",
        Action::DeletePage => "delete-page",
        Action::InsertRemind => "insert-remind",
        Action::InsertRemindNag => "insert-remind-nag",
        Action::OpenReminders => "open-reminders",
        Action::OpenTaskList => "open-task-list",
        Action::SnoozeReminder => "snooze-reminder",
        Action::AddProperty => "add-property",
        Action::OpenProperties => "open-properties",
        // `pin` is the TUI's `PinCommand`.
        Action::TogglePin => "pin",
        Action::CutBlock => "cut-block",
        Action::CopyBlock => "copy-block",
        Action::PasteBlock => "paste-block",
        Action::ExitInsert => "exit-insert",
        Action::CommitAndContinue => "commit-and-continue",
        Action::DeleteEmptyBlock => "delete-empty-block",

        // ── visual / range ───────────────────────────────────────
        Action::EnterVisual => "enter-visual",
        Action::YankCurrentBlock => "yank-current-block",
        Action::YankRange => "yank-range",
        Action::DeleteRange => "delete-range",
        Action::SelectRangeDown => "select-range-down",
        Action::SelectRangeUp => "select-range-up",
        Action::MoveVisualRangeUp => "move-visual-range-up",
        Action::MoveVisualRangeDown => "move-visual-range-down",

        // ── code / inline markdown ───────────────────────────────
        // `run` is the TUI's `RunCommand`.
        Action::RunCodeBlock => "run",
        Action::WrapBold => "wrap-bold",
        Action::WrapItalic => "wrap-italic",
        Action::WrapCode => "wrap-code",
        Action::WrapStrike => "wrap-strike",
        Action::InsertLink => "insert-link",
        Action::Undo => "undo",
        Action::Redo => "redo",
    }
}

#[cfg(test)]
mod tests {
    use super::command_name;
    use crate::action::Action;

    /// The reason this module exists: a user who learned `/today` in
    /// the TUI types it on the desktop and reaches the same thing.
    #[test]
    fn the_tui_names_are_the_catalog_names() {
        for (action, name) in [
            (Action::OpenToday, "today"),
            (Action::TogglePin, "pin"),
            (Action::ToggleHelp, "help"),
            (Action::RunCodeBlock, "run"),
            (Action::CopyBlockRef, "refer"),
            (Action::OpenPicker, "open"),
            (Action::Quit, "quit"),
        ] {
            assert_eq!(command_name(action), name, "{action:?}");
        }
    }

    /// A `/` menu dispatches on this string, so two actions under one
    /// name is a row whose behaviour depends on `match` order.
    #[test]
    fn no_two_actions_share_a_name() {
        let mut seen: Vec<(&str, Action)> = Vec::new();
        for action in Action::ALL {
            let name = command_name(*action);
            if let Some((_, other)) = seen.iter().find(|(n, _)| *n == name) {
                panic!("`{name}` names both {other:?} and {action:?}");
            }
            seen.push((name, *action));
        }
    }

    /// The name rides a `/` trigger whose alphabet stops at the first
    /// character outside `[a-z0-9-]`, so anything else is a name the
    /// user cannot finish typing.
    #[test]
    fn every_name_is_typeable_after_a_slash() {
        for action in Action::ALL {
            let name = command_name(*action);
            assert!(!name.is_empty(), "{action:?} has an empty name");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{action:?} is named `{name}`, which is not slash-typeable"
            );
        }
    }
}
