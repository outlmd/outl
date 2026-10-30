//! Overlay key router and the four overlay-specific handlers.
//!
//! Overlays are modal: while one is open, Normal/Insert/Visual mode
//! handlers don't run — every key goes through here.
//!
//! - **QuickSwitch** — fuzzy page/journal picker (`Ctrl+P`).
//! - **Search** — workspace text search (via `/search` slash command).
//! - **Command** — vim-style `:command` palette.
//! - **Slash** — Notion-style `/` menu, surface for built-in and
//!   plugin commands.

use super::is_typed_text;
use crate::state::{App, Overlay};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Route a keystroke to whichever overlay is currently open.
///
/// Returns `Ok(true)` when the caller should exit the event loop.
pub(crate) fn handle_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match &app.overlay {
        Some(Overlay::QuickSwitch(_)) => handle_quick_switch_key(app, key),
        Some(Overlay::Search(_)) => handle_search_overlay_key(app, key),
        Some(Overlay::Command(_)) => handle_command_overlay_key(app, key),
        Some(Overlay::Slash(_)) => handle_slash_overlay_key(app, key),
        Some(Overlay::TemplatePicker(_)) => handle_template_picker_key(app, key),
        Some(Overlay::Reminders(_)) => handle_reminders_key(app, key),
        Some(Overlay::PluginSettings(_)) => handle_plugin_settings_key(app, key),
        Some(Overlay::Properties(_)) => handle_properties_key(app, key),
        Some(Overlay::SyncPanel(_)) => crate::actions::sync_panel::handle_key(app, key),
        Some(Overlay::Error(_)) => {
            // Modal error popup: any key dismisses. Special-case Ctrl+C
            // so it still quits the whole TUI.
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(true);
            }
            app.overlay = None;
            Ok(false)
        }
        None => Ok(false),
    }
}

fn handle_quick_switch_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => app.accept_quick_switch()?,
        KeyCode::Up => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                qs.selected = qs.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                if qs.selected + 1 < qs.candidates.len() {
                    qs.selected += 1;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                qs.query.pop();
            }
            app.refresh_quick_switch();
        }
        KeyCode::Char(c) if is_typed_text(key.modifiers) => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                qs.query.push(c);
            }
            app.refresh_quick_switch();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_search_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => app.accept_search()?,
        KeyCode::Up => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                s.selected = s.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                if s.selected + 1 < s.hits.len() {
                    s.selected += 1;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                s.query.pop();
            }
            app.refresh_search();
        }
        KeyCode::Char(c) if is_typed_text(key.modifiers) => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                s.query.push(c);
            }
            app.refresh_search();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_command_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => {
            let buf = if let Some(Overlay::Command(ref c)) = app.overlay {
                c.buffer.clone()
            } else {
                return Ok(false);
            };
            app.overlay = None;
            return run_command(app, &buf);
        }
        KeyCode::Backspace => {
            if let Some(Overlay::Command(ref mut c)) = app.overlay {
                c.buffer.pop();
            }
        }
        KeyCode::Char(ch) if is_typed_text(key.modifiers) => {
            if let Some(Overlay::Command(ref mut c)) = app.overlay {
                c.buffer.push(ch);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_slash_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => return app.accept_slash(),
        KeyCode::Up => {
            app.slash_select_prev();
        }
        KeyCode::Down => {
            app.slash_select_next();
        }
        KeyCode::Backspace => {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.query.pop();
            }
            app.refresh_slash();
        }
        KeyCode::Char(c) if is_typed_text(key.modifiers) => {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.query.push(c);
            }
            app.refresh_slash();
        }
        _ => {}
    }
    Ok(false)
}

/// Execute a `:command` from the command bar. Returns `Ok(true)` when
/// the command quits the app.
///
/// Routes everything through the `command_registry`. The vim palette
/// and the `/` slash menu share that registry, so a plugin that
/// registers a new command shows up in both surfaces without code
/// duplication here.
fn run_command(app: &mut App, line: &str) -> Result<bool> {
    let registry = app.command_registry.clone();
    registry.dispatch(app, line)
}

fn handle_template_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => return app.accept_template_picker(),
        KeyCode::Up => {
            if let Some(Overlay::TemplatePicker(ref mut tp)) = app.overlay {
                tp.selected = tp.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Overlay::TemplatePicker(ref mut tp)) = app.overlay {
                if tp.selected + 1 < tp.filtered.len() {
                    tp.selected += 1;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(Overlay::TemplatePicker(ref mut tp)) = app.overlay {
                tp.query.pop();
            }
            app.refresh_template_picker();
        }
        KeyCode::Char(c) if is_typed_text(key.modifiers) => {
            if let Some(Overlay::TemplatePicker(ref mut tp)) = app.overlay {
                tp.query.push(c);
            }
            app.refresh_template_picker();
        }
        _ => {}
    }
    Ok(false)
}

/// Reminders overlay keys. Read-only navigation plus the two actions
/// that make sense without leaving the list: snooze and jump.
///
/// No `d` (DONE): rewriting a block from a list that hides it is how
/// you check off the wrong row.
fn handle_reminders_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.overlay = None,
        KeyCode::Enter => app.open_selected_reminder(),
        // The same three options the GUI clients render, in the same
        // order, resolved by the same `SnoozePreset`.
        KeyCode::Char('s') => app.snooze_selected_reminder(0),
        KeyCode::Char('t') => app.snooze_selected_reminder(1),
        KeyCode::Char('w') => app.snooze_selected_reminder(2),
        KeyCode::Up | KeyCode::Char('k') => app.move_reminders_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_reminders_cursor(1),
        _ => {}
    }
    Ok(false)
}

/// Properties overlay keys.
///
/// Two key streams behind one `match`: navigating the list is vim
/// (`j` / `k` / `d d` / `o` / `q`), and editing a field is a plain
/// line editor (`Enter` advances or commits, `Esc` aborts). A form
/// with `Tab` between fields would fight both, which is why `Tab`
/// here means "complete", never "next field".
fn handle_properties_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Ctrl+C still quits the whole TUI, even mid-edit.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    let editing = matches!(
        &app.overlay,
        Some(Overlay::Properties(p)) if p.editing.is_some()
    );

    if !editing {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.close_properties(),
            KeyCode::Up | KeyCode::Char('k') => app.properties_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.properties_move(1),
            KeyCode::Enter => app.properties_begin_edit(),
            KeyCode::Char('o') => app.properties_begin_new(),
            // `d d`, same chord (and the same "arm, then fire") the
            // outline uses to delete a block.
            KeyCode::Char('d') => app.properties_pending_delete(),
            // `p` flips block ⇄ page. The page's properties were
            // reachable only through `:prop-page`, which needs you to
            // already know the key — the exact gap this overlay exists
            // to close.
            KeyCode::Char('p') => app.toggle_properties_scope(),
            _ => {}
        }
        return Ok(false);
    }

    // The `[[` / `#` popup owns navigation while it is up, exactly as
    // it does in Insert mode.
    if app.autocomplete.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.autocomplete = None;
                return Ok(false);
            }
            KeyCode::Enter | KeyCode::Tab => {
                app.accept_property_value_completion();
                return Ok(false);
            }
            KeyCode::Up => {
                if let Some(ac) = &mut app.autocomplete {
                    ac.selected = ac.selected.saturating_sub(1);
                }
                return Ok(false);
            }
            KeyCode::Down => {
                if let Some(ac) = &mut app.autocomplete {
                    if ac.selected + 1 < ac.candidates.len() {
                        ac.selected += 1;
                    }
                }
                return Ok(false);
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => app.properties_cancel_edit(),
        KeyCode::Enter => app.properties_advance_or_commit(),
        KeyCode::Tab => app.properties_complete_key(),
        KeyCode::Backspace => app.properties_edit_backspace(),
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.properties_edit_push(c);
        }
        _ => {}
    }
    Ok(false)
}

fn handle_plugin_settings_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Ctrl+C still quits the whole TUI, even mid-edit.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    let editing = matches!(
        &app.overlay,
        Some(Overlay::PluginSettings(ps)) if ps.editing.is_some()
    );

    if editing {
        match key.code {
            KeyCode::Esc => app.plugin_settings_cancel_edit(),
            KeyCode::Enter => app.plugin_settings_commit_edit(),
            KeyCode::Backspace => app.plugin_settings_edit_backspace(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.plugin_settings_edit_push(c);
            }
            _ => {}
        }
    } else {
        match key.code {
            KeyCode::Esc => app.overlay = None,
            KeyCode::Up => app.plugin_settings_move(-1),
            KeyCode::Down => app.plugin_settings_move(1),
            KeyCode::Enter => app.plugin_settings_activate(),
            _ => {}
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
