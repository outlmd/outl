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
        KeyCode::Char(c) => {
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
        KeyCode::Char(c) => {
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
        KeyCode::Char(ch) => {
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
        KeyCode::Char(c) => {
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
        KeyCode::Char(c) => {
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
/// `d` (mark DONE) from the issue's sketch is deliberately absent for
/// now — completing a task rewrites the block's text, and doing that
/// from a list the user can't see the full block in is how you check
/// off the wrong row.
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
mod properties_overlay_tests {
    //! The `g p` overlay driven through the real key router.
    //!
    //! These press keys rather than calling `App` methods directly:
    //! the overlay's whole point is the key stream (`o`, `Tab`, `dd`,
    //! `p`), and a test that skips the router proves the actions work
    //! while the chord that reaches them is misspelled.

    use super::handle_overlay_key;
    use crate::state::{App, Overlay, PropertyField, PropertyScope};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new(
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
        (app, dir)
    }

    fn press(app: &mut App, code: KeyCode) {
        handle_overlay_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn state(app: &App) -> &crate::state::PropertiesState {
        match &app.overlay {
            Some(Overlay::Properties(p)) => p,
            other => panic!("expected the properties overlay, got {other:?}"),
        }
    }

    #[test]
    fn o_then_a_key_and_a_value_writes_the_property() {
        // The whole point of the issue: creating the *first* property
        // on a block, without knowing `:prop`.
        let (mut app, _dir) = fresh_app();
        app.open_properties();

        press(&mut app, KeyCode::Char('o'));
        type_str(&mut app, "status");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "shipped");
        press(&mut app, KeyCode::Enter);

        assert_eq!(
            app.property_on_current_block("status").as_deref(),
            Some("shipped")
        );
        assert_eq!(state(&app).rows, vec![("status".into(), "shipped".into())]);
    }

    #[test]
    fn tab_completes_the_key_from_what_the_workspace_already_uses() {
        // Keys repeat in a real graph; retyping `related` is the
        // friction this overlay exists to remove.
        let (mut app, _dir) = fresh_app();
        app.open_properties();
        press(&mut app, KeyCode::Char('o'));
        // Seed the catalogue by hand — a fresh in-memory workspace has
        // no properties yet, and the ranking itself is `known_keys`'
        // own tested contract.
        if let Some(Overlay::Properties(ref mut p)) = app.overlay {
            p.known_keys = vec!["related".into(), "remind".into()];
            if let Some(edit) = p.editing.as_mut() {
                edit.key_matches = p.known_keys.clone();
            }
        }

        press(&mut app, KeyCode::Tab);
        assert_eq!(state(&app).editing.as_ref().unwrap().key, "related");

        // Tab again cycles rather than sticking on the first match.
        press(&mut app, KeyCode::Tab);
        assert_eq!(state(&app).editing.as_ref().unwrap().key, "remind");
    }

    #[test]
    fn enter_on_a_row_edits_the_value_and_leaves_the_key_alone() {
        let (mut app, _dir) = fresh_app();
        app.set_property_on_current_block("status", "draft");
        app.open_properties();

        press(&mut app, KeyCode::Enter);
        assert_eq!(
            state(&app).editing.as_ref().unwrap().field,
            PropertyField::Value,
            "editing an existing row must land in the value, not the key"
        );
        for _ in 0..5 {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "shipped");
        press(&mut app, KeyCode::Enter);

        assert_eq!(
            app.property_on_current_block("status").as_deref(),
            Some("shipped")
        );
        assert_eq!(
            app.page.blocks[0].properties.len(),
            1,
            "the edit must replace the pair, not append a second one"
        );
    }

    #[test]
    fn dd_deletes_the_highlighted_property_and_a_single_d_does_not() {
        // Same "arm, then fire" contract the outline's `d d` has: one
        // stray `d` must never destroy a property.
        let (mut app, _dir) = fresh_app();
        app.set_property_on_current_block("status", "draft");
        app.open_properties();

        press(&mut app, KeyCode::Char('d'));
        assert_eq!(
            app.property_on_current_block("status").as_deref(),
            Some("draft"),
            "one `d` only arms the chord"
        );

        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.property_on_current_block("status"), None);
        assert!(state(&app).rows.is_empty());
    }

    #[test]
    fn moving_the_cursor_disarms_a_pending_delete() {
        // `d` then `j` then `d` must not delete: the second `d` is the
        // first half of a new chord, aimed at a different row.
        let (mut app, _dir) = fresh_app();
        app.set_property_on_current_block("status", "draft");
        app.set_property_on_current_block("icon", "🚀");
        app.open_properties();

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('d'));

        assert_eq!(app.page.blocks[0].properties.len(), 2, "nothing deleted");
    }

    #[test]
    fn p_switches_to_the_pages_properties_and_writes_there() {
        // `:prop-page` was the only door to these, and it needs you to
        // know the key already.
        let (mut app, _dir) = fresh_app();
        app.open_properties();
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(state(&app).scope, PropertyScope::Page);

        press(&mut app, KeyCode::Char('o'));
        type_str(&mut app, "icon");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "🚀");
        press(&mut app, KeyCode::Enter);

        assert!(
            app.page
                .properties
                .iter()
                .any(|(k, v)| k == "icon" && v == "🚀"),
            "expected the page property, got {:?}",
            app.page.properties
        );
        assert!(
            app.page.blocks[0].properties.is_empty(),
            "the block must not have been touched"
        );
    }

    #[test]
    fn a_structural_page_key_is_refused_not_written() {
        // `page-slug` / `page-kind` are the page's identity. Writing
        // one as a property renames the page out from under every ref
        // pointing at it.
        let (mut app, _dir) = fresh_app();
        app.open_properties();
        press(&mut app, KeyCode::Char('p'));
        press(&mut app, KeyCode::Char('o'));
        type_str(&mut app, "page-slug");
        press(&mut app, KeyCode::Enter);

        let s = state(&app);
        assert_eq!(
            s.editing.as_ref().unwrap().field,
            PropertyField::Key,
            "the refusal must keep the caret in the key field"
        );
        assert!(
            s.message
                .as_deref()
                .unwrap_or("")
                .contains("defines the page"),
            "a silent refusal reads as a broken Enter"
        );
        assert!(app.page.properties.iter().all(|(k, _)| k != "page-slug"));
    }

    #[test]
    fn a_key_with_no_value_typed_yet_is_refused_rather_than_written_empty() {
        // An empty value is the *delete* path in both writers, so
        // committing a blank key would be a no-op that looks like a
        // successful add.
        let (mut app, _dir) = fresh_app();
        app.open_properties();
        press(&mut app, KeyCode::Char('o'));
        press(&mut app, KeyCode::Enter);

        assert_eq!(
            state(&app).editing.as_ref().unwrap().field,
            PropertyField::Key
        );
        assert!(state(&app)
            .message
            .as_deref()
            .unwrap_or("")
            .contains("cannot be empty"));
    }

    #[test]
    fn two_open_brackets_in_the_value_fire_the_page_picker() {
        // 87% of property values in a real graph are a `[[page]]` or a
        // `#tag`. If the trigger doesn't reach the value field, the
        // overlay is a worse `:prop`.
        let (mut app, _dir) = fresh_app();
        let parsed = outl_md::parse::parse("title:: Avelino\n\n- hi\n");
        app.index
            .patch_page(std::path::Path::new("pages/avelino.md"), &parsed);
        app.open_properties();

        press(&mut app, KeyCode::Char('o'));
        type_str(&mut app, "related");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "[[Ave");

        let ac = app
            .autocomplete
            .as_ref()
            .expect("`[[` must open the same popup Insert mode uses");
        assert_eq!(ac.kind, crate::state::AutocompleteKind::PageRef);
        assert!(ac.candidates.iter().any(|c| c == "Avelino"));

        // Tab accepts it into the value, closing the brackets.
        press(&mut app, KeyCode::Tab);
        assert!(app.autocomplete.is_none());
        assert_eq!(
            state(&app).editing.as_ref().unwrap().value,
            "[[Avelino]]",
            "accepting must complete the ref, not append beside it"
        );
    }

    #[test]
    fn closing_the_overlay_takes_the_autocomplete_popup_with_it() {
        // The popup renders from `App::autocomplete`, independent of
        // the overlay. Leaving it set paints a dangling box over the
        // outline with no keystream to close it.
        let (mut app, _dir) = fresh_app();
        let parsed = outl_md::parse::parse("title:: Avelino\n\n- hi\n");
        app.index
            .patch_page(std::path::Path::new("pages/avelino.md"), &parsed);
        app.open_properties();
        press(&mut app, KeyCode::Char('o'));
        type_str(&mut app, "related");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "[[Ave");
        assert!(app.autocomplete.is_some());

        // Esc closes the popup, Esc closes the edit, Esc closes the
        // overlay — one level per press, like every modal here.
        press(&mut app, KeyCode::Esc);
        assert!(app.autocomplete.is_none());
        press(&mut app, KeyCode::Esc);
        assert!(state(&app).editing.is_none());
        press(&mut app, KeyCode::Esc);
        assert!(app.overlay.is_none());
        assert!(app.autocomplete.is_none());
    }
}
