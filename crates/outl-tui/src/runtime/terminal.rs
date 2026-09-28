//! Terminal state this process owns globally: whether stdout is one,
//! and how to hand it back when the program dies unexpectedly.
//!
//! The alt-screen / raw-mode dance around a *successful* run lives in
//! the parent module, next to the boot sequence it brackets. What is
//! here is the part that has to work when nothing else does.

use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};

pub(super) fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Install a process-wide panic hook that restores the terminal before
/// chaining to the previous handler.
///
/// Without this, a panic mid-render leaves the user staring at a
/// garbled terminal in raw mode with no cursor — they have to `reset`
/// blind to recover. Calling this is idempotent in spirit (only the
/// first call chains the real default hook).
pub(super) fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Restore the terminal state. Errors are ignored — we're
        // already panicking; nothing useful to do with a second
        // failure.
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = execute!(std::io::stdout(), crossterm::cursor::Show);
        previous(info);
    }));
}
