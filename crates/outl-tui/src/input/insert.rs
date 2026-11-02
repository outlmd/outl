//! Insert-mode key handler: text input, soft newlines, cross-block
//! Left/Right and Up/Down nav, autocomplete intercept, paste-pair
//! collapsing.
//!
//! Insert is the only mode where keystrokes mutate the in-flight
//! `EditBuffer` directly. Structural keys (`Enter`, `Tab`,
//! `Backspace` on empty) still commit through the regular `App`
//! methods so the op log stays the single source of truth.

use crate::actions::block::insert::InsertCursor;
use crate::actions::cycle_todo_inline;
use crate::state::{App, Mode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{cross_block_nav_eligible, cross_block_step, cursor_inside_open_fence, is_typed_text};

pub(crate) fn handle_insert_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Autocomplete intercepts navigation/Enter/Esc.
    if app.autocomplete.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.autocomplete = None;
                return Ok(());
            }
            KeyCode::Enter => {
                app.accept_autocomplete();
                return Ok(());
            }
            KeyCode::Tab => {
                app.accept_autocomplete();
                return Ok(());
            }
            KeyCode::Up => {
                if let Some(ac) = &mut app.autocomplete {
                    ac.selected = ac.selected.saturating_sub(1);
                }
                return Ok(());
            }
            KeyCode::Down => {
                if let Some(ac) = &mut app.autocomplete {
                    if !ac.candidates.is_empty() && ac.selected + 1 < ac.candidates.len() {
                        ac.selected += 1;
                    }
                }
                return Ok(());
            }
            _ => {} // fall through to the regular handler
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.commit_insert();
        }
        // `Ctrl+Enter` toggles TODO directly inside the buffer — no
        // commit, no new block, cursor preserved relative to text.
        // `Ctrl+T` is the portable alias for terminals/multiplexers
        // (tmux, Terminal.app) that collapse `Ctrl+Enter` into plain
        // `Enter`.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                cycle_todo_inline(buffer);
            }
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                cycle_todo_inline(buffer);
            }
        }
        // Soft newline inside the same block.
        //
        // Three ways to trigger it:
        //   1. `Shift+Enter` — kitty-protocol-aware terminals only
        //      (kitty, wezterm, alacritty, Ghostty, foot, iTerm2 ≥ 3.5).
        //      Older terminals (Terminal.app, plain xterm) collapse it
        //      into `Enter`.
        //   2. `Alt+Enter` / `Ctrl+J` — portable fallbacks. Alt depends
        //      on the terminal's "Option as Meta" setting on macOS;
        //      `Ctrl+J` (ASCII LF) always works.
        //   3. **Auto-detect**: plain `Enter` while inside an *open*
        //      fenced code block (a `` ``` `` opener without a matching
        //      closer above the cursor) — keeps users typing
        //      ```lisp\n(+ 1 2)\n``` without any modifier at all.
        //
        // The kitty-protocol push happens in [`crate::runtime`].
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.insert_char('\n');
            }
        }
        // `Ctrl+J` = ASCII LF, distinguishable from `Enter` (CR) in
        // every terminal. Last-resort portable soft newline. Must come
        // before `Ctrl+...` matches that handle other letters.
        KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.insert_char('\n');
            }
        }
        KeyCode::Enter => {
            // Plain Enter — `\n` inside an open code fence, sibling
            // block otherwise.
            let in_fence = matches!(&app.mode, Mode::Insert { buffer, .. }
                if cursor_inside_open_fence(&buffer.as_string(), buffer.cursor));
            if in_fence {
                if let Mode::Insert { buffer, .. } = &mut app.mode {
                    buffer.insert_char('\n');
                }
            } else {
                app.create_block_below();
            }
        }
        KeyCode::Tab => {
            app.indent_current();
        }
        KeyCode::BackTab => {
            app.outdent_current();
        }
        KeyCode::Backspace => {
            let should_delete = if let Mode::Insert { buffer, .. } = &mut app.mode {
                if buffer.cursor == 0 && buffer.is_empty() {
                    true
                } else if !buffer.delete_pair_back() {
                    // No empty `[[]]` / `(())` to collapse around the
                    // cursor — fall back to deleting the previous char.
                    buffer.delete_back();
                    false
                } else {
                    false
                }
            } else {
                false
            };
            if should_delete {
                app.abort_insert();
                app.delete_current();
                app.move_selection(-1);
            }
        }
        KeyCode::Delete => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.delete_forward();
            }
        }
        KeyCode::Left => {
            // Inside the buffer (including across `\n` in a multi-line
            // block), just decrement the char cursor. At buffer start
            // (cursor == 0), spill into the *previous* block and place
            // the cursor at its end — so holding Left walks the user
            // back through the document like a single document.
            let at_start = matches!(&app.mode, Mode::Insert { buffer, .. } if buffer.cursor == 0);
            if at_start && app.selected > 0 {
                app.commit_insert();
                app.move_selection(-1);
                app.enter_insert(InsertCursor::AtCursor); // cursor lands at end
            } else if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_left();
            }
        }
        KeyCode::Right => {
            // Symmetric of Left: at buffer end, spill into the next
            // block with the cursor at its start.
            let at_end = matches!(&app.mode, Mode::Insert { buffer, .. }
                if buffer.cursor == buffer.len());
            let last_idx = app.flat_len.saturating_sub(1);
            if at_end && app.selected < last_idx {
                app.commit_insert();
                app.move_selection(1);
                app.enter_insert(InsertCursor::Start); // cursor at start
            } else if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_right();
            }
        }
        // Up / Down cross blocks the same way Left/Right do — the
        // outline reads as one continuous document, so a user who
        // hits `Down` at the bottom of a block lands inside the next
        // one without ever leaving Insert. Multi-line buffers (fenced
        // code) absorb the move internally first, falling back to
        // cross-block nav only when the cursor is already on the
        // buffer's first/last line.
        //
        // Scope today: outline blocks edited in `CurrentPage`. Cross-
        // page backlink editing (`EditTarget::SourcePage`) keeps the
        // older Esc → j/k → i flow — adding cross-page Up/Down would
        // also need to commit the cross-page write per keystroke, and
        // the trade-offs there deserve their own pass.
        KeyCode::Up => {
            let moved_in_buffer = if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_up()
            } else {
                false
            };
            if !moved_in_buffer && cross_block_nav_eligible(app) && app.selected > 0 {
                cross_block_step(app, -1);
            }
        }
        KeyCode::Down => {
            let moved_in_buffer = if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_down()
            } else {
                false
            };
            let last_idx = app.flat_len.saturating_sub(1);
            if !moved_in_buffer && cross_block_nav_eligible(app) && app.selected < last_idx {
                cross_block_step(app, 1);
            }
        }
        KeyCode::Home => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_home();
            }
        }
        KeyCode::End => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_end();
            }
        }
        KeyCode::Char(ch) if is_typed_text(key.modifiers) => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                match ch {
                    '(' => buffer.insert_pair('(', ')'),
                    '[' => buffer.insert_pair('[', ']'),
                    '{' => buffer.insert_pair('{', '}'),
                    _ => buffer.insert_char(ch),
                }
            }
        }
        _ => {}
    }
    // After any key, re-check whether the cursor is inside a `[[` or `#`
    // token. Cheap because page count is small.
    app.maybe_update_autocomplete();
    Ok(())
}

#[cfg(test)]
mod modifier_guard_tests {
    //! A `Ctrl`-chord must not be typed into the block being edited.
    //!
    //! `Ctrl+W` arrives as `Char('w')` plus CONTROL (issue #429), and
    //! this is the most-typed-in surface in the client, so an
    //! unguarded arm corrupts the user's text on the muscle memory
    //! every terminal user has.

    use super::handle_insert_key;
    use crate::actions::block::InsertCursor;
    use crate::state::{App, Mode};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn app_editing_a_block() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        app.page.blocks.clear();
        app.page.blocks.push(outl_md::parse::OutlineNode {
            text: "ship it".to_string(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
        // Park the caret past the last character so a typed char
        // appends, which keeps the assertions below readable.
        app.cursor_col = "ship it".chars().count();
        app.enter_insert(InsertCursor::AtCursor);
        (app, dir)
    }

    fn buffer_text(app: &App) -> String {
        match &app.mode {
            Mode::Insert { buffer, .. } => buffer.as_string(),
            other => panic!("expected Insert mode, got {other:?}"),
        }
    }

    fn chord(app: &mut App, c: char, mods: KeyModifiers) {
        handle_insert_key(app, KeyEvent::new(KeyCode::Char(c), mods)).unwrap();
    }

    #[test]
    fn ctrl_w_does_not_land_in_the_block() {
        let (mut app, _d) = app_editing_a_block();
        chord(&mut app, 'w', KeyModifiers::CONTROL);
        assert_eq!(
            buffer_text(&app),
            "ship it",
            "Ctrl+W wrote into the block instead of being read as a chord"
        );
    }

    #[test]
    fn the_whole_readline_set_stays_out_of_the_block() {
        let (mut app, _d) = app_editing_a_block();
        for c in ['a', 'e', 'w', 'u', 'k', 'b', 'f'] {
            chord(&mut app, c, KeyModifiers::CONTROL);
        }
        assert_eq!(buffer_text(&app), "ship it");
    }

    #[test]
    fn ordinary_typing_still_reaches_the_block() {
        let (mut app, _d) = app_editing_a_block();
        chord(&mut app, '!', KeyModifiers::NONE);
        chord(&mut app, 'A', KeyModifiers::SHIFT);
        assert_eq!(
            buffer_text(&app),
            "ship it!A",
            "the guard must not swallow ordinary typing"
        );
    }

    #[test]
    fn ctrl_t_still_cycles_todo_because_its_arm_comes_first() {
        let (mut app, _d) = app_editing_a_block();
        chord(&mut app, 't', KeyModifiers::CONTROL);
        assert_eq!(
            buffer_text(&app),
            "TODO ship it",
            "the guard must not shadow the Ctrl+T arm above it"
        );
    }
}
