//! Key-event routing: turn a [`crossterm::event::KeyEvent`] into a
//! method call on `crate::state::App`.
//!
//! Each `handle_*_key` function pattern-matches a key, decides which
//! action to invoke, and returns. They never render; they never read
//! or write files directly; they delegate everything to methods on
//! `App` (defined in [`crate::actions`]).
//!
//! Keymap *documentation* (the table users care about) lives in
//! [`crate::keymap`].
//!
//! ## Multi-line editing inside a block (Insert mode)
//!
//! `Alt+Enter` inserts a soft newline — the cursor stays inside the
//! same block, so users can write multi-line bullets and fenced code
//! blocks (e.g. ` ```lisp `…` ``` `). Plain `Enter` commits the block
//! and creates a sibling below.
//!
//! Terminals that speak the kitty keyboard protocol (kitty, wezterm,
//! alacritty, Ghostty, foot, recent iTerm2) also accept `Shift+Enter`
//! for the same action; older terminals (Terminal.app, plain xterm)
//! collapse `Shift+Enter` into `Enter`, which is why `Alt+Enter` is
//! the portable fallback. The keyboard-enhancement push happens in
//! [`crate::runtime`].
//!
//! ## Module layout
//!
//! | Submodule  | What's in it                                                 |
//! |------------|--------------------------------------------------------------|
//! | `normal`   | `handle_normal_key` — selection, chords, mode switches       |
//! | `insert`   | `handle_insert_key` — text edits, soft newlines, cross-block |
//! | `overlay`  | `handle_overlay_key` + quick-switch / search / command / slash |
//! | `visual`   | `handle_visual_key` — range ops over selected blocks         |
//!
//! Shared helpers (`cross_block_step`, `cross_block_nav_eligible`,
//! `cursor_inside_open_fence`) live here in `mod.rs` so every
//! submodule can `use super::*`.

use crate::state::{App, EditTarget, Focus, Mode};
use crossterm::event::KeyModifiers;

mod chord_adapter;
mod insert;
mod normal;
mod overlay;
mod plugin_chord;
mod visual;

pub(crate) use insert::handle_insert_key;
pub(crate) use normal::handle_normal_key;
pub(crate) use overlay::handle_overlay_key;
pub(crate) use visual::handle_visual_key;

/// Cross-block Up/Down nav only kicks in when:
///
/// - We're in Insert on the current page (not a source-page backlink
///   edit — that has its own commit semantics).
/// - Focus is on the outline (the inline backlinks panel is its own
///   navigable surface).
///
/// Backlink editing keeps the older Esc → j/k → i workflow until the
/// cross-page write story gets a dedicated pass — committing a
/// source-page edit per keystroke triggers a full reconcile, and the
/// trade-offs aren't obvious enough to bake in here.
pub(super) fn cross_block_nav_eligible(app: &App) -> bool {
    matches!(
        &app.mode,
        Mode::Insert {
            target: EditTarget::CurrentPage,
            ..
        }
    ) && matches!(app.focus, Focus::Outline)
}

/// Commit the current buffer and move the outline selection by
/// `delta` (`-1` for Up, `+1` for Down), then re-enter Insert on the
/// new block preserving the cursor's preferred column.
///
/// **Backlinks guard:** when `move_selection` crosses past the outline
/// (the inline backlinks panel is the next navigable zone), it lands
/// `Focus` on `Focus::Backlink`. In that case we **stop in Normal
/// mode** instead of dispatching to `enter_insert_backlink`, which
/// would silently open a *different file* (the source page of the
/// backlink) for editing. The user pressed Down expecting more of
/// the same document, not to start editing some other page — they
/// can step into the backlink panel intentionally with `Esc → j`.
pub(super) fn cross_block_step(app: &mut App, delta: i32) {
    let pref_col = if let Mode::Insert { buffer, .. } = &app.mode {
        buffer.visual_column()
    } else {
        0
    };
    app.commit_insert();
    app.move_selection(delta);
    if !matches!(app.focus, Focus::Outline) {
        // Crossed into the backlinks panel (or any future non-outline
        // focus). Stay in Normal — opening another page mid-Insert
        // surprises the user.
        return;
    }
    app.cursor_col = pref_col;
    app.enter_insert(crate::actions::block::InsertCursor::AtCursor);
}

/// Is a `KeyCode::Char` carrying these modifiers a character the user
/// meant to type, rather than a chord this client does not handle?
///
/// Every `Char` arm that writes into text has to ask. crossterm
/// reports `Ctrl+W` as `Char('w')` plus [`KeyModifiers::CONTROL`], so
/// an arm that matches `Char(ch)` and inserts it turns the whole
/// readline set into literal letters.
pub(super) fn is_typed_text(mods: KeyModifiers) -> bool {
    // An allowlist, so a modifier this crate has not seen yet is read
    // as a chord rather than as text. Getting that default wrong writes
    // into the user's block; getting it wrong the other way drops a
    // keystroke, which they can see.
    mods.difference(KeyModifiers::SHIFT).is_empty()
}

/// Is `cursor` (char index) sitting inside an *open* fenced code block?
///
/// "Open" here is from the *Enter-key perspective*: would pressing
/// Enter right now keep the cursor inside a fence?
///
/// Counts ` ``` ` markers on every line up to and **including** the
/// cursor's line. A `` ``` `` opener on the cursor's own line opens the
/// fence: the very next Enter starts the body. A `` ``` `` closer on
/// the cursor's line *also* counts — once the user types the closing
/// fence, the next Enter exits.
pub(super) fn cursor_inside_open_fence(text: &str, cursor: usize) -> bool {
    let cursor_line = text.chars().take(cursor).filter(|&c| c == '\n').count();
    let mut open = false;
    for (idx, line) in text.split('\n').enumerate() {
        if idx > cursor_line {
            break;
        }
        if line.trim_start().starts_with("```") {
            open = !open;
        }
    }
    open
}

#[cfg(test)]
mod tests {
    use super::{cursor_inside_open_fence, is_typed_text};
    use crossterm::event::KeyModifiers;

    #[test]
    fn plain_typing_is_text() {
        assert!(is_typed_text(KeyModifiers::NONE));
    }

    #[test]
    fn shift_is_text_so_capitals_still_type() {
        assert!(is_typed_text(KeyModifiers::SHIFT));
    }

    #[test]
    fn a_control_chord_is_not_text() {
        // `Ctrl+W` arrives as `Char('w')` + CONTROL. Treating it as
        // text is issue #429: the readline set lands in the block.
        assert!(!is_typed_text(KeyModifiers::CONTROL));
    }

    #[test]
    fn an_alt_chord_is_not_text() {
        // Alt is Meta in a terminal, and this crate already binds
        // `Alt+Enter` and `Alt+Up`/`Alt+Down`.
        assert!(!is_typed_text(KeyModifiers::ALT));
    }

    #[test]
    fn a_super_chord_is_not_text() {
        assert!(!is_typed_text(KeyModifiers::SUPER));
    }

    #[test]
    fn control_plus_shift_is_not_text() {
        assert!(!is_typed_text(KeyModifiers::CONTROL | KeyModifiers::SHIFT));
    }

    #[test]
    fn empty_buffer_is_not_inside_fence() {
        assert!(!cursor_inside_open_fence("", 0));
    }

    #[test]
    fn plain_text_is_not_inside_fence() {
        let t = "first line\nsecond line";
        assert!(!cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_inside_open_fence_after_opener() {
        // `` ```lisp\n<cursor> ``
        let t = "```lisp\n";
        assert!(cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_inside_open_fence_after_opener_and_body() {
        let t = "```lisp\n(+ 1 2)\n";
        assert!(cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_after_closed_fence_is_outside() {
        // Both opener and closer are above the cursor.
        let t = "```lisp\n(+ 1 2)\n```\n";
        assert!(!cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_on_opener_line_is_inside() {
        // User typed ` ```lisp ` and is about to press Enter to start
        // the body. From the Enter-key perspective, the fence is now
        // open: the next newline must be a soft newline.
        let t = "```lisp";
        assert!(cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_on_closer_line_is_outside() {
        // User just finished typing the closing `` ``` ``. From the
        // Enter-key perspective the closer already balanced the
        // opener — the next Enter should exit the block, not insert
        // another soft newline.
        let t = "```lisp\n(+ 1 2)\n```";
        assert!(!cursor_inside_open_fence(t, t.chars().count()));
    }
}
