//! The frames before there is a workspace.
//!
//! `run_with_theme_override` used to open the workspace **first** and
//! enter the alternate screen afterwards, so everything the open costs
//! was spent against a terminal still showing the user's shell prompt:
//! 190-270ms on a warm 10k-page workspace, and the whole of a cold
//! replay on a large one. Nothing said the app was starting, and on a
//! slow open the only visible difference between "outl is replaying a
//! 700k-op log" and "outl hung" was that one of them eventually drew.
//!
//! The order is inverted now: raw mode, the alternate screen and a
//! skeleton frame come first, then the open. The user sees outl
//! immediately and sees *what it is doing*.
//!
//! # The keyboard-enhancement query is the other half
//!
//! `supports_keyboard_enhancement()` writes a CSI sequence and waits
//! for the terminal to answer. A terminal that does not answer costs
//! crossterm's internal timeout, measured at **2.4s on a launch** — and
//! during it the app accepts no keys, because the query owns stdin.
//!
//! It cannot move to a background thread: it *reads stdin*, so a thread
//! doing that races the event loop for the user's keystrokes, and the
//! failure mode of losing that race is swallowed input. A short timeout
//! has the same problem — the answer still arrives, into whoever reads
//! next.
//!
//! So it moved **after the first real frame** instead
//! ([`query_enhanced_keys`]). On a terminal that answers, nothing
//! changes. On one that does not, the user is looking at their journal
//! for those 2.4s rather than at a shell prompt, and the keys that
//! query enables (`Shift+Enter`, `Ctrl+Enter`) have a documented
//! portable fallback (`Alt+Enter`) for exactly the terminals that do
//! not answer it.
//!
//! **What is still owed**: those 2.4s are input-dead wherever they
//! happen. Closing that needs a way to skip the query outright, which
//! is a config key and a decision about its default, not a reordering.

use std::io::Stdout;
use std::path::Path;

use crossterm::execute;
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Terminal;

use crate::theme::Theme;

/// What the skeleton says it is doing. One line, present tense, and the
/// workspace path, because the most useful thing a slow launch can tell
/// you is *which* workspace it is slow on.
const OPENING: &str = "Opening";

/// Draw the pre-workspace frame: the app's name, what it is doing, and
/// which workspace.
///
/// Deliberately not a stripped-down `render_app`. That one takes an
/// `App`, and an `App` needs a `Workspace` — which is the thing this
/// frame exists to be shown *instead of waiting for*. A second renderer
/// is the cost; it is nine lines and it cannot drift into claiming
/// anything about the document, because it has no document to read.
pub(super) fn draw_skeleton(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    theme: &Theme,
    path: &Path,
) {
    let canvas = Style::default().bg(theme.background).fg(theme.foreground);
    // A failed draw is not worth aborting a launch over: the real
    // frames come from the event loop, which reports its own errors.
    let _ = terminal.draw(|f| {
        let area = f.area();
        let lines = vec![
            Line::from(Span::styled("outl", theme.heading)),
            Line::from(""),
            Line::from(Span::styled(
                format!("{OPENING} {}", path.display()),
                theme.dim,
            )),
        ];
        let body = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default().style(canvas));
        f.render_widget(body, area);
    });
}

/// Ask the terminal whether it speaks the kitty keyboard protocol, and
/// push the flags when it does. Returns whether they were pushed (the
/// caller pops them on the way out).
///
/// **Call this after the first real frame**, never before — the module
/// doc carries the 2.4s that buys.
pub(super) fn query_enhanced_keys(backend: &mut CrosstermBackend<Stdout>) -> bool {
    // Terminals that don't support it (Terminal.app, plain xterm)
    // silently ignore the CSI sequence; we still have Alt+Enter as a
    // portable fallback.
    let supported = supports_keyboard_enhancement().unwrap_or(false);
    if supported {
        let _ = execute!(
            backend,
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | crossterm::event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
            )
        );
    }
    supported
}
