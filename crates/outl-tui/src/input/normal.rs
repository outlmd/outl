//! Normal-mode key handler: outline navigation, structural ops,
//! chord recognition, mode switches, and the in-Normal sidebar +
//! help intercepts.
//!
//! Three layers of dispatch run before the main `match key.code`:
//! the help-popup intercept (swallows every key but tab switches),
//! the sidebar intercept (when focus is inside the sidebar), and
//! the chord accumulator (`d`/`g`/`y`/`q` arm for a follow-up key).
//! Everything past the chord block is bare-key handling.

use crate::actions::block::InsertCursor;
use crate::state::{App, PendingInputOp};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn handle_normal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Plugin keybindings get first crack — but `try_plugin_binding`
    // refuses to shadow any native action (see `super::plugin_chord`),
    // so this never steals a key the user expects to do its built-in
    // thing. We must not fire while a help popup, sidebar focus, or a
    // pending native op/chord is active, so guard on those first.
    if !app.show_help
        && app.sidebar_focus.is_none()
        && app.pending_input_op.is_none()
        && app.pending_chord.is_none()
        && super::plugin_chord::try_plugin_binding(app, key)
    {
        return Ok(false);
    }
    // Help popup owns the keyboard exclusively while open. Any key
    // that isn't a tab switch / scroll / close is *swallowed* — we
    // don't want `j` to move the outline behind the popup while the
    // user thinks they're scrolling help.
    if app.show_help {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                if app.help_tab == 0 {
                    app.help_tab = crate::view::HELP_TABS.len() - 1;
                } else {
                    app.help_tab -= 1;
                }
                app.help_scroll = 0; // new tab → top
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => {
                app.help_tab = (app.help_tab + 1) % crate::view::HELP_TABS.len();
                app.help_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.help_scroll = app.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.help_scroll = app.help_scroll.saturating_sub(1);
            }
            KeyCode::PageDown => {
                app.help_scroll = app.help_scroll.saturating_add(10);
            }
            KeyCode::PageUp => {
                app.help_scroll = app.help_scroll.saturating_sub(10);
            }
            KeyCode::Char('g') | KeyCode::Home => {
                app.help_scroll = 0;
            }
            KeyCode::Char('G') | KeyCode::End => {
                // Big number — the renderer clamps against the
                // actual body length when it draws, so we don't need
                // to know the count here.
                app.help_scroll = u16::MAX / 2;
            }
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                app.show_help = false;
                app.help_scroll = 0;
            }
            _ => {
                // Swallow every other key. The popup has focus; the
                // outline behind it must not react.
            }
        }
        return Ok(false);
    }

    // Sidebar intercept: while focus is inside the sidebar, j/k
    // navigate the focused section, Tab cycles sections, Enter opens
    // the item, Esc returns focus to the outline (sidebar stays
    // visible). `\` always closes the sidebar entirely, handled
    // further down in the Normal handler.
    //
    // `d` arms a one-shot "delete this page?" confirmation; the next
    // keystroke resolves it inside the same intercept — `y` / `Y`
    // confirms, anything else cancels (and is swallowed, matching
    // the `pending_input_op` contract).
    if app.sidebar_focus.is_some() || app.pending_sidebar_delete.is_some() {
        // Delete confirmation: takes priority over regular sidebar
        // navigation so the user can't accidentally move the cursor
        // while the "delete? y/n" prompt is up.
        if app.pending_sidebar_delete.is_some() {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    app.sidebar_confirm_delete()?;
                    return Ok(false);
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                    app.pending_sidebar_delete = None;
                    app.status.clear();
                    return Ok(false);
                }
                _ => {
                    // Swallow every other key — the prompt is modal.
                    // (Same posture as `pending_input_op`: cancel on
                    // any non-confirming input rather than routing it.)
                    app.pending_sidebar_delete = None;
                    app.status.clear();
                    return Ok(false);
                }
            }
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                app.sidebar_move(1);
                return Ok(false);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.sidebar_move(-1);
                return Ok(false);
            }
            KeyCode::Char('g') => {
                app.sidebar_cursor = 0;
                return Ok(false);
            }
            KeyCode::Char('G') => {
                app.sidebar_move(i32::MAX / 2);
                return Ok(false);
            }
            KeyCode::Tab => {
                app.sidebar_cycle_section(true);
                return Ok(false);
            }
            KeyCode::BackTab => {
                app.sidebar_cycle_section(false);
                return Ok(false);
            }
            KeyCode::Enter => {
                app.sidebar_activate()?;
                return Ok(false);
            }
            KeyCode::Char('d') => {
                app.sidebar_delete_current();
                return Ok(false);
            }
            KeyCode::Esc => {
                app.sidebar_blur();
                return Ok(false);
            }
            KeyCode::Char('e' | 'E') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+E (Ctrl+Shift+E too — most terminals collapse
                // them) is the same toggle that opens the sidebar
                // from Normal; pressing it while focused closes.
                // Matches the desktop's `Cmd+Shift+E`.
                app.sidebar_close();
                return Ok(false);
            }
            _ => {}
        }
    }

    // Pending input op: `r` / `f` / `F` armed a one-shot waiting for
    // the next char. Resolves before chord/bare matching so a literal
    // `r{q}` doesn't trip the `qq` chord arm.
    if let Some(op) = app.pending_input_op.take() {
        if let KeyCode::Char(ch) = key.code {
            match op {
                PendingInputOp::ReplaceChar => app.replace_char_under_cursor(ch),
                PendingInputOp::FindCharForward => app.find_char_forward(ch),
                PendingInputOp::FindCharBackward => app.find_char_backward(ch),
            }
        } else {
            // Non-char (Esc, arrows, …) cancels the pending op. Clear
            // the prompt status so a stale `r… (replace)` / `f… (find →)`
            // doesn't linger in the footer until the next status write.
            app.status.clear();
        }
        return Ok(false);
    }

    // Chord handling: a previous 'd' / 'g' / 'y' is pending.
    if let Some(pending) = app.pending_chord.take() {
        match (pending, key.code) {
            ('d', KeyCode::Char('d')) => {
                app.delete_current();
                return Ok(false);
            }
            ('g', KeyCode::Char('j')) => {
                app.go_today()?;
                return Ok(false);
            }
            ('g', KeyCode::Char('x')) => {
                // `gx` = run the code block under the cursor.
                // Mnemonic: "go execute".
                app.run_current_block();
                return Ok(false);
            }
            ('g', KeyCode::Char('g')) => {
                // `gg` = jump to the first block (vim convention).
                app.move_selection(i32::MIN / 2);
                return Ok(false);
            }
            ('g', KeyCode::Char('p')) => {
                // `g p` = open the property editor. "go properties".
                // The discoverable half of `:prop`: the command needs
                // you to remember the key, the overlay lists what the
                // block already has and completes new keys from the
                // workspace catalogue.
                app.open_properties();
                return Ok(false);
            }
            ('g', KeyCode::Char('P')) => {
                // `g P` = toggle the `pinned::` page property. It used
                // to be `g p`; properties took that spelling because
                // pinning is a once-per-page act with `/pin` as its
                // other door, while `pinned::` is itself one of the
                // page properties `g p` now edits. Shifted for the
                // rarer of the two, same as `g r` / `g R`.
                app.toggle_pinned();
                return Ok(false);
            }
            ('g', KeyCode::Char('d')) => {
                // `gd` = delete the focused page. "go delete".
                // Mirrors `Action::DeletePage` in the shared shortcut
                // catalog. Two targets: the sidebar-highlighted row
                // when the sidebar owns focus, otherwise the current
                // page. Both paths funnel through
                // `sidebar_delete_current` (which arms a `y/n`
                // confirmation) — the outline branch pre-fills the
                // pending state with the current slug so the same
                // confirm handler runs.
                app.delete_page_from_chord();
                return Ok(false);
            }
            ('g', KeyCode::Char('r')) => {
                // `g r` = attach a starter `remind::` to this block.
                // "go remind". Non-destructive: a block that already
                // has a rule reports it instead of overwriting.
                app.insert_remind();
                return Ok(false);
            }
            ('g', KeyCode::Char('R')) => {
                // `g R` = the "nag me" preset in one chord
                // (`remind:: now every 1h until DONE`). Shifted because
                // it DOES overwrite an existing rule — escalating is an
                // explicit act, not a fat-fingered `g r`.
                app.insert_remind_nag();
                return Ok(false);
            }
            ('g', KeyCode::Char('s')) => {
                // `g s` = snooze the selected block's reminder 1h,
                // without opening the overlay. Converges via
                // `Op::SnoozeRemind`, so the phone goes quiet too.
                app.snooze_selected_block();
                return Ok(false);
            }
            ('g', KeyCode::Char('n')) => {
                // `g n` = open the reminders overlay. "go
                // notifications". The issue's sketch put this on
                // `Ctrl+R`, but that is already Redo — and a terminal
                // can't tell `Ctrl+R` from `Ctrl+Shift+R`, so the `g`
                // family is the honest home for it here. The desktop
                // still takes `Cmd/Ctrl+Shift+R`.
                app.open_reminders();
                return Ok(false);
            }
            ('y', KeyCode::Char('y')) => {
                app.yank_current();
                return Ok(false);
            }
            ('y', KeyCode::Char('r')) => {
                // `yr` — yank the **block ref handle** of the
                // currently selected block. Surfaces `((blk-XXXXXX))`
                // in the status line and stashes it in
                // `last_yanked_ref` so a later paste/insert command
                // can use it.
                app.yank_current_ref();
                return Ok(false);
            }
            ('q', KeyCode::Char('q')) => {
                // Confirmed quit. The first `q` armed the chord; the
                // second within one keystroke window seals the deal.
                return Ok(true);
            }
            ('Z', KeyCode::Char('Z')) => {
                // `ZZ` — vim's "save and quit". The TUI commits Insert
                // on every boundary already (Esc, structural ops), so
                // by the time we land in Normal mode the on-disk state
                // is current. `ZZ` therefore reduces to "quit" here —
                // alias for `qq`. Kept distinct so muscle memory from
                // vim users works without surprise.
                return Ok(true);
            }
            ('g', KeyCode::Char('v')) => {
                // `gv` — re-enter Visual at the last range. No-op (with
                // a status message) when no Visual session has happened.
                app.reselect_last_visual();
                return Ok(false);
            }
            ('z', KeyCode::Char('R')) => {
                // `zR` — unfold every block on the page (vim "reduce
                // folding to nothing").
                app.unfold_all();
                return Ok(false);
            }
            ('z', KeyCode::Char('M')) => {
                // `zM` — fold every block on the page (vim "more
                // folding").
                app.fold_all();
                return Ok(false);
            }
            ('z', KeyCode::Char('z')) => {
                // `zz` — center the viewport on the cursor.
                app.center_viewport_on_selection();
                return Ok(false);
            }
            ('z', KeyCode::Char('i')) => {
                // `zi` — zoom in: focus the selected block as the render
                // root (Roam/Workflowy). `Action::ZoomIn` in the shared
                // catalog.
                app.zoom_in_on_selected();
                return Ok(false);
            }
            ('z', KeyCode::Char('o')) => {
                // `zo` — zoom out one level. `Action::ZoomOut`.
                app.zoom_out();
                return Ok(false);
            }
            _ => {} // fall through to normal handling
        }
    }

    match key.code {
        KeyCode::Char('q') => {
            // Don't quit on a single `q` — too easy to hit by
            // accident, takes the user out of their editor with no
            // way to recover. Arm a chord instead; a *second* `q`
            // (or `:quit` / `Ctrl+C`) closes the TUI.
            app.pending_chord = Some('q');
            app.status = "press q again to quit".into();
            return Ok(false);
        }
        KeyCode::Char('Z') => {
            // Vim's `ZZ` ("save and quit"). Arms a chord; a second
            // capital `Z` confirms. Saving is implicit — every Insert
            // commit already flushes the buffer to disk before we get
            // back to Normal, so by the time the chord matters there
            // is nothing left to save.
            app.pending_chord = Some('Z');
            app.status = "press Z again to save and quit".into();
            return Ok(false);
        }
        KeyCode::Char('?') => app.show_help = !app.show_help,
        // `Ctrl+T` is the portable alias for `Ctrl+Enter` (TODO toggle)
        // — tmux without `extended-keys` and Terminal.app collapse
        // `Ctrl+Enter` to plain `Enter`, so the chord we *want* never
        // arrives. Must come BEFORE the bare `t` arm.
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => app.toggle_todo(),
        // `t` and `Home` were sharing one binding: `t` jumps to today,
        // `Home` should move the cursor to the start of the current
        // block. Split them.
        KeyCode::Char('t') => app.go_today()?,
        KeyCode::Char('[') => app.shift_journal(-1)?,
        KeyCode::Char(']') => app.shift_journal(1)?,
        KeyCode::Char('g') => app.pending_chord = Some('g'),
        // `z` arms the fold-control chord family (`zR`, `zM`, `zz`).
        // Single `z` does nothing on its own; the next key resolves
        // it in the chord block above.
        KeyCode::Char('z') => {
            app.pending_chord = Some('z');
            app.status = "z…".into();
        }
        // Half-page jumps must be matched *before* the plain `d`/`u`
        // chord arms — match guards are tried in order and a guard-less
        // `Char('d')` arm would win otherwise.
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.move_selection((app.viewport_height.max(2) / 2) as i32)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.move_selection(-((app.viewport_height.max(2) / 2) as i32))
        }
        KeyCode::Char('d') => app.pending_chord = Some('d'),
        KeyCode::Char('y') => app.pending_chord = Some('y'),
        // Fold / unfold the selected block. The renderer's triangle
        // marker (▶/▼) is the visual confirmation. No-op when the
        // block has no sidecar entry yet (see
        // `App::toggle_collapse_selected`).
        KeyCode::Char('c') => app.toggle_collapse_selected(),
        // Paste the OS clipboard. `p` pastes WITH formatting (outline /
        // paragraph conversion), `P` (Shift+P) pastes WITHOUT formatting
        // (raw text as one block). Plain `p` only — Ctrl+P is the quick
        // switcher and must beat this. The internal yank register is
        // still pasted, but copy/yank now mirrors it to the OS clipboard,
        // so `p` reads the same content the register holds.
        KeyCode::Char('p') if key.modifiers.is_empty() => app.paste_clipboard_formatted(),
        KeyCode::Char('P') if key.modifiers == KeyModifiers::SHIFT || key.modifiers.is_empty() => {
            app.paste_clipboard_plain()
        }
        KeyCode::Tab => {
            app.indent_current();
        }
        KeyCode::BackTab => {
            app.outdent_current();
        }
        // `Ctrl+Enter` toggles TODO. Must come BEFORE the plain `Enter`
        // arm or the open-ref handler eats the chord.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => app.toggle_todo(),
        // Enter is overloaded: if the cursor is sitting on a `[[ref]]`
        // / `#tag` / journal date, open it. Otherwise enter Insert
        // mode (the original behavior).
        KeyCode::Enter if !app.try_open_under_cursor()? => {
            app.enter_insert(InsertCursor::AtCursor);
        }
        KeyCode::Enter => {}
        KeyCode::Char('i') => app.enter_insert(InsertCursor::AtCursor),
        KeyCode::Char('I') => app.enter_insert(InsertCursor::Start),
        // `a` (append) — enter Insert one char to the right of the
        // cursor, vim-style. Clamped at end of buffer so `a` at the
        // last position behaves identically to `i` there.
        KeyCode::Char('a') => app.enter_insert(InsertCursor::AfterCursor),
        // `A` (append at end) — `$` then `i`. Single keypress for the
        // common "jump to end and start typing" gesture.
        KeyCode::Char('A') => app.enter_insert_at_end(),
        // Flip the backlinks list direction (newest ⇄ oldest, issue
        // #142). `Ctrl+O` — "O" for order — mirrors the `Ctrl+B` panel
        // toggle and persists the choice to `config.toml`. Must come
        // **before** the bare `Char('o')` / `Char('O')` below (open
        // block below / above): those arms are unguarded, so once Rust
        // reaches them the modifier is lost and they'd swallow Ctrl+O.
        KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_backlinks_order()
        }
        KeyCode::Char('o') => app.create_block_below(),
        KeyCode::Char('O') => app.create_block_above(),
        // ── Vim char / line ops ──────────────────────────────────────
        // Each one mutates `page.blocks` directly and routes through
        // `save()` — same write path as structural ops.
        KeyCode::Char('x') => app.delete_char_under_cursor(),
        KeyCode::Char('X') => app.delete_char_before_cursor(),
        KeyCode::Char('D') => app.delete_to_end_of_block(),
        KeyCode::Char('C') => app.change_to_end_of_block(),
        KeyCode::Char('S') => app.substitute_block(),
        KeyCode::Char('s') => app.substitute_char(),
        // `r{ch}` — arm a one-shot: the next char replaces the char
        // under the cursor without entering Insert.
        KeyCode::Char('r') if key.modifiers.is_empty() => {
            app.pending_input_op = Some(PendingInputOp::ReplaceChar);
            app.status = "r… (replace)".into();
        }
        KeyCode::Char('f') if key.modifiers.is_empty() => {
            app.pending_input_op = Some(PendingInputOp::FindCharForward);
            app.status = "f… (find →)".into();
        }
        KeyCode::Char('F') => {
            app.pending_input_op = Some(PendingInputOp::FindCharBackward);
            app.status = "F… (find ←)".into();
        }
        KeyCode::Char('~') => app.toggle_case_under_cursor(),
        // `Y` — alias of `yy`. vim's "yank line" / outl's "yank block".
        KeyCode::Char('Y') => app.yank_current_alias(),
        // `e` — cursor to the end of the next word. Guarded so it
        // doesn't shadow `Ctrl+E` (sidebar toggle) below.
        KeyCode::Char('e') if key.modifiers.is_empty() => app.cursor_word_end(),
        // `*` / `#` — search the workspace for the word under cursor,
        // forward / backward. `n` / `N` walk through the results
        // afterwards (the same persisted `last_search` the `/` overlay
        // populates).
        KeyCode::Char('*') => app.search_word_under_cursor(true)?,
        KeyCode::Char('#') => app.search_word_under_cursor(false)?,
        // Vertical navigation: blocks. `j`/`k` are vim conventions.
        // Alt + arrows drag the current block (more discoverable than capital K/J).
        KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => app.move_block_up(),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => app.move_block_down(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        // Page-sized jumps. `viewport_height` is set by the renderer
        // on each draw, so it tracks the actual terminal size.
        KeyCode::PageDown => app.move_selection(app.viewport_height.max(1) as i32),
        KeyCode::PageUp => app.move_selection(-(app.viewport_height.max(1) as i32)),
        // `G` = last block (vim convention).
        KeyCode::Char('G') => app.move_selection(i32::MAX / 2),
        // Horizontal cursor inside the current block.
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor_col(-1),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor_col(1),
        KeyCode::Char('0') | KeyCode::Home => app.cursor_to_home(),
        KeyCode::Char('$') | KeyCode::End => app.cursor_to_end(),
        // Toggle backlinks panel. `Ctrl+B` (Ctrl+Shift+B too — most
        // terminals collapse them). Mirrors the desktop's
        // `Cmd+Shift+B`; both clients hide backlinks by default and
        // open them on demand.
        //
        // Must come **before** the unconditional `Char('b')` (vim
        // word-left) below — Rust matches arms top-to-bottom and a
        // pattern guard can't recover the modifier branch once an
        // earlier unguarded arm captures the bare char.
        KeyCode::Char('b' | 'B') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.show_backlinks = !app.show_backlinks
        }
        // Toggle the left sidebar (mini-calendar, pinned, recent).
        // Default off — `Ctrl+E` opts in. Matches the desktop's
        // `Cmd+Shift+E` (VS Code's "show explorer" chord). Most
        // terminals collapse `Ctrl+Shift+E` into `Ctrl+E`, so we
        // match either letter case with the CONTROL modifier and
        // both feel identical to the user.
        //
        // Why not `\`? It clashed with desktop standardisation —
        // single source of truth for the chrome chord lives in
        // `outl-shortcuts`, and the desktop's `Cmd+Shift+E` is the
        // industry-standard "toggle sidebar" mapping (VS Code,
        // Cursor).
        //
        // Opening jumps focus straight to the first non-empty
        // section (Pinned by default), so the user can immediately
        // `j/k` through items and `Enter` to open — no extra Tab
        // to "enter" the sidebar.
        KeyCode::Char('e' | 'E') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.show_sidebar {
                app.sidebar_close();
            } else {
                app.sidebar_open_focused();
            }
        }
        KeyCode::Char('w') => app.cursor_word_right(),
        KeyCode::Char('b') => app.cursor_word_left(),
        // Block reordering (vim-ish: capital J/K drag the block).
        KeyCode::Char('K') => app.move_block_up(),
        KeyCode::Char('J') => app.move_block_down(),
        // History.
        KeyCode::Char('u') => app.undo(),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => app.redo(),
        // Overlays.
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.open_quick_switch();
            app.refresh_quick_switch();
        }
        // `/` opens the Notion-style slash command menu. Workspace
        // search lives there too (`/search`) — one extra keystroke,
        // full discoverability, future plugin commands appear
        // automatically.
        KeyCode::Char('/') => {
            app.open_slash();
            app.refresh_slash();
        }
        KeyCode::Char(':') => app.open_command(),
        // Walk through the last `/` search results without reopening
        // the overlay. `n` next, `N` previous (vim convention).
        KeyCode::Char('n') => app.search_next()?,
        KeyCode::Char('N') => app.search_prev()?,
        // Enter Visual mode (vim-style: V selects entire blocks).
        KeyCode::Char('V') => app.enter_visual(),
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
mod reminder_chord_tests {
    //! Drift guard between the shared chord catalog and this file.
    //!
    //! The TUI dispatches Normal-mode keys through its own `match` here
    //! rather than `outl_shortcuts::lookup`, so the catalog and the
    //! match arms can disagree without anything failing to compile.
    //! These tests pin the reminder chords on the catalog side; if
    //! someone re-spells one there, this fails and points at the arm
    //! that needs the same edit.
    //!
    //! They do not prove the arm *runs* (that needs a live `App` with a
    //! workspace). They prove the two halves still agree on the
    //! spelling, which is the half that silently rots.

    use outl_shortcuts::{lookup, Action, Chord, ChordSequence, Key, Mode, Modifiers};

    fn pair(a: char, b: char) -> ChordSequence {
        ChordSequence::pair(Chord::ch(a), Chord::ch(b))
    }

    #[test]
    fn g_r_still_means_insert_remind() {
        assert_eq!(
            lookup(Mode::Normal, &pair('g', 'r')),
            Some(Action::InsertRemind),
            "the `('g', Char('r'))` arm in handle_normal_key must match this"
        );
    }

    #[test]
    fn g_shift_r_still_means_the_nag_preset() {
        let seq = ChordSequence::pair(Chord::ch('g'), Chord::new(Modifiers::SHIFT, Key::char('r')));
        assert_eq!(
            lookup(Mode::Normal, &seq),
            Some(Action::InsertRemindNag),
            "the `('g', Char('R'))` arm in handle_normal_key must match this"
        );
    }

    #[test]
    fn g_n_still_opens_the_reminders_overlay() {
        assert_eq!(
            lookup(Mode::Normal, &pair('g', 'n')),
            Some(Action::OpenReminders),
            "the `('g', Char('n'))` arm in handle_normal_key must match this"
        );
    }

    #[test]
    fn g_s_still_snoozes_the_selected_block() {
        assert_eq!(
            lookup(Mode::Normal, &pair('g', 's')),
            Some(Action::SnoozeReminder),
            "the `('g', Char('s'))` arm in handle_normal_key must match this"
        );
    }

    #[test]
    fn g_p_opens_the_property_editor() {
        assert_eq!(
            lookup(Mode::Normal, &pair('g', 'p')),
            Some(Action::OpenProperties),
            "the `('g', Char('p'))` arm in handle_normal_key must match this"
        );
    }

    #[test]
    fn g_shift_p_is_the_pin_toggle_that_g_p_displaced() {
        // `g p` meant pin until the property editor took the
        // spelling. If someone reverts one half of that move, the
        // other half silently becomes unreachable — this is the test
        // that notices.
        let seq = ChordSequence::pair(Chord::ch('g'), Chord::new(Modifiers::SHIFT, Key::char('p')));
        assert_eq!(
            lookup(Mode::Normal, &seq),
            Some(Action::TogglePin),
            "the `('g', Char('P'))` arm in handle_normal_key must match this"
        );
    }

    #[test]
    fn ctrl_p_is_still_the_quick_switcher_not_properties() {
        // `g p` is a two-key chord; a bare `Ctrl+P` must keep opening
        // the picker, which is the muscle memory most users have.
        assert_eq!(
            lookup(Mode::Normal, &ChordSequence::chord(Chord::ctrl('p'))),
            Some(Action::OpenPicker)
        );
    }

    #[test]
    fn ctrl_r_is_still_redo_not_reminders() {
        // The issue sketched `Ctrl+R` for the reminders overlay. It is
        // Redo, and a terminal can't tell it from `Ctrl+Shift+R`, which
        // is why the TUI took `g n` instead. If this ever flips, the
        // user loses undo/redo to a list nobody asked to open.
        assert_eq!(
            lookup(Mode::Normal, &ChordSequence::chord(Chord::ctrl('r'))),
            Some(Action::Redo)
        );
    }
}
