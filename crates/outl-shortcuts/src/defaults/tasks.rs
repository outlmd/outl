//! Task list rows. Same owner as `defaults.rs`, split out only
//! because that file sits at its size ceiling.

use super::chords::{pair, shift_ctrl_ch, shift_meta_ch};
use crate::action::Action;
use crate::binding::{Binding, Mode};

/// `g t` in Normal and `Cmd/Ctrl+Shift+L` everywhere open the task
/// list, beside `g n` / `Cmd/Ctrl+Shift+R` for reminders.
///
/// Both pairs were free in this catalog. `T` was the first choice for
/// the global chord and was refused: the TUI's `Ctrl+T` arm toggles a
/// TODO, and a terminal without the kitty protocol collapses
/// `Ctrl+Shift+T` onto it, so a chord meant to open a list would edit
/// a block. `L` collapses onto cursor-right, which moves and nothing
/// more.
pub(super) fn bindings() -> Vec<Binding> {
    vec![
        Binding::new(
            pair('g', 't'),
            Mode::Normal,
            Action::OpenTaskList,
            "Open the task list (chord)",
        ),
        Binding::new(
            shift_meta_ch('l'),
            Mode::Global,
            Action::OpenTaskList,
            "Open the task list (Cmd+Shift+L)",
        ),
        Binding::new(
            shift_ctrl_ch('l'),
            Mode::Global,
            Action::OpenTaskList,
            "Open the task list (Ctrl+Shift+L)",
        ),
    ]
}
