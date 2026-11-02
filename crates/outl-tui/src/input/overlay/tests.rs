//! Tests for the overlay key router.
//!
//! Split out of `overlay.rs` to keep that file under the size ratchet
//! (`scripts/check-file-size.sh`); the module layout follows
//! `view/outline.rs` + `view/outline/tests.rs`.
//!
//! Both modules press keys through the real router rather than calling
//! `App` methods: an overlay's whole point is its key stream, so a test
//! that skips the routing proves the actions work while the chord that
//! reaches them is misspelled.

#[cfg(test)]
mod properties_overlay_tests {
    //! The `g p` overlay driven through the real key router.
    //!
    //! These press keys rather than calling `App` methods directly:
    //! the overlay's whole point is the key stream (`o`, `Tab`, `dd`,
    //! `p`), and a test that skips the router proves the actions work
    //! while the chord that reaches them is misspelled.

    use crate::input::overlay::handle_overlay_key;
    use crate::state::{App, Overlay, PropertyField, PropertyScope};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
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

#[cfg(test)]
mod modifier_guard_tests {
    //! A `Ctrl`-chord must not be typed into an overlay's query.
    //!
    //! crossterm reports `Ctrl+W` as `Char('w')` plus CONTROL, so an
    //! arm matching `Char(c)` and pushing it turns the readline set
    //! into literal letters in the search box (issue #429).
    //!
    //! These press keys through the real router rather than poking the
    //! query string: the bug *is* the routing, so a test that skips it
    //! proves nothing.

    use crate::input::overlay::handle_overlay_key;
    use crate::state::{App, Overlay};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    fn chord(app: &mut App, c: char, mods: KeyModifiers) {
        handle_overlay_key(app, KeyEvent::new(KeyCode::Char(c), mods)).unwrap();
    }

    /// Every query an overlay owns, by the opener that creates it.
    fn query_of(app: &App) -> &str {
        match &app.overlay {
            Some(Overlay::QuickSwitch(qs)) => &qs.query,
            Some(Overlay::Search(s)) => &s.query,
            Some(Overlay::Command(c)) => &c.buffer,
            Some(Overlay::Slash(s)) => &s.query,
            Some(Overlay::TemplatePicker(tp)) => &tp.query,
            other => panic!("expected a query overlay, got {other:?}"),
        }
    }

    #[test]
    fn ctrl_w_is_not_typed_into_the_quick_switcher() {
        let (mut app, _d) = fresh_app();
        app.open_quick_switch();
        chord(&mut app, 'w', KeyModifiers::CONTROL);
        assert_eq!(
            query_of(&app),
            "",
            "Ctrl+W landed in the query; the readline set is being read as text"
        );
    }

    #[test]
    fn ctrl_w_is_not_typed_into_the_search_box() {
        let (mut app, _d) = fresh_app();
        app.open_search();
        chord(&mut app, 'w', KeyModifiers::CONTROL);
        assert_eq!(query_of(&app), "");
    }

    #[test]
    fn ctrl_u_is_not_typed_into_the_command_palette() {
        let (mut app, _d) = fresh_app();
        app.open_command();
        chord(&mut app, 'u', KeyModifiers::CONTROL);
        assert_eq!(query_of(&app), "");
    }

    #[test]
    fn ctrl_a_is_not_typed_into_the_slash_menu() {
        let (mut app, _d) = fresh_app();
        app.open_slash();
        chord(&mut app, 'a', KeyModifiers::CONTROL);
        assert_eq!(query_of(&app), "");
    }

    #[test]
    fn plain_and_shifted_characters_still_reach_the_query() {
        let (mut app, _d) = fresh_app();
        app.open_quick_switch();
        chord(&mut app, 'o', KeyModifiers::NONE);
        chord(&mut app, 'K', KeyModifiers::SHIFT);
        assert_eq!(
            query_of(&app),
            "oK",
            "the guard must not swallow ordinary typing"
        );
    }
}
