//! Where a property write goes, and the one refusal it can come back
//! with.
//!
//! Three doors onto the same two writers: the `g p` overlay's Block and
//! Page scopes, and `/prop-page <key> <value>`. They are here rather
//! than in `actions/properties` (the overlay) or
//! `commands/builtins/page` (the command) because the routing is the
//! part they must not each have their own copy of — `title::` was the
//! proof, forked in the overlay and not in the command, which left the
//! half-rename reachable by typing the other one.

use crate::state::{App, Overlay, PropertyScope};

impl App {
    /// Route the `g p` overlay's write to its scope's owner.
    ///
    /// Empty `value` deletes in both, which is why the overlay's `d d`
    /// needs no second writer. `Err` is a refusal the caller shows
    /// through [`Self::show_property_refusal`].
    pub(crate) fn write_property(&mut self, key: &str, value: &str) -> Result<(), String> {
        let Some(Overlay::Properties(p)) = &self.overlay else {
            return Ok(());
        };
        if p.scope == PropertyScope::Page {
            return self.write_page_property(key, value);
        }
        self.set_property_on_current_block(key, value);
        Ok(())
    }

    /// Write a page-level property, with `title::` routed through the
    /// rename.
    ///
    /// `title` is half of the page's identity: `page_meta` resolves the
    /// page's display name from it and every `[[old name]]` in the
    /// workspace resolves through it. Writing it into the AST changed
    /// the name and respelled nothing — the silent half-rename that
    /// `page-slug`'s own refusal ("rename the page instead") already
    /// points away from, pointing at a feature this client did not have
    /// until `/rename` landed. [`Self::retitle_current_page`] is that
    /// rename, and the slug deliberately does not move: a title is not
    /// a filename.
    pub(crate) fn write_page_property(&mut self, key: &str, value: &str) -> Result<(), String> {
        if key.eq_ignore_ascii_case(outl_actions::page::TITLE_KEY) {
            return self.retitle_current_page(value);
        }
        self.set_property_on_page(key, value);
        Ok(())
    }

    /// Put a refused write's reason on the overlay's own message line.
    ///
    /// It also disarms a pending `d d`: the chord was aimed at a row
    /// the write declined to touch, and leaving it armed makes the next
    /// keystroke delete a different property.
    pub(crate) fn show_property_refusal(&mut self, why: String) {
        if let Some(Overlay::Properties(ref mut p)) = self.overlay {
            p.pending_delete = false;
            p.message = Some(why);
        }
    }
}

#[cfg(test)]
mod tests {
    //! The `title::` chip is a rename, and these live beside
    //! `write_page_property` because that is the single owner of the
    //! fork. They used to sit in `page_rename/tests.rs` only because
    //! the `App` fixture did; it is `page_test_support` now.

    use crate::actions::page_test_support::{fresh_app, seed};

    #[test]
    fn the_title_chip_respells_mentions_instead_of_half_renaming() {
        let (mut app, _dir) = fresh_app();
        seed(&mut app);

        app.retitle_current_page("Gamma").unwrap();

        let beta = std::fs::read_to_string(app.workspace_root.join("pages/beta.md")).unwrap();
        assert!(
            beta.contains("[[Gamma]]"),
            "editing the title used to change the name and respell nothing:\n{beta}"
        );
        assert_eq!(
            app.current_slug(),
            "alpha",
            "a title is not a filename — the slug stays put"
        );
        let alpha = std::fs::read_to_string(app.workspace_root.join("pages/alpha.md")).unwrap();
        assert!(alpha.contains("title:: Gamma"), "{alpha}");
    }

    #[test]
    fn the_title_chip_goes_through_the_overlay_too() {
        let (mut app, _dir) = fresh_app();
        seed(&mut app);
        app.open_properties();
        app.toggle_properties_scope();
        app.properties_begin_new();
        for c in "title".chars() {
            app.properties_edit_push(c);
        }
        app.properties_advance_or_commit();
        for c in "Gamma".chars() {
            app.properties_edit_push(c);
        }
        app.properties_advance_or_commit();

        let beta = std::fs::read_to_string(app.workspace_root.join("pages/beta.md")).unwrap();
        assert!(
            beta.contains("[[Gamma]]"),
            "the chip is wired to the rename, not to `set_property_on_page`:\n{beta}"
        );
    }

    #[test]
    fn prop_page_title_is_the_same_write_and_not_a_second_one() {
        // The overlay's Page scope and `/prop-page` are two doors onto one
        // write, and only one of them used to have the fork — so the
        // half-rename stayed reachable by typing the other.
        let (mut app, _dir) = fresh_app();
        seed(&mut app);

        let registry = app.command_registry.clone();
        registry
            .dispatch(&mut app, "prop-page title Gamma")
            .unwrap();

        let beta = std::fs::read_to_string(app.workspace_root.join("pages/beta.md")).unwrap();
        assert!(beta.contains("[[Gamma]]"), "{beta}");
    }

    #[test]
    fn prop_page_still_writes_an_ordinary_property() {
        let (mut app, _dir) = fresh_app();
        seed(&mut app);

        let registry = app.command_registry.clone();
        registry.dispatch(&mut app, "prop-page icon 🚀").unwrap();

        assert!(app
            .page
            .properties
            .iter()
            .any(|(k, v)| k == "icon" && v == "🚀"));
    }

    #[test]
    fn emptying_the_title_chip_is_refused_rather_than_leaving_the_page_nameless() {
        let (mut app, _dir) = fresh_app();
        seed(&mut app);
        let before = app.workspace.log().len();

        let refusal = app.retitle_current_page("   ").expect_err("refused");

        assert!(
            refusal.contains("/rename"),
            "it names the way out: {refusal}"
        );
        assert_eq!(app.workspace.log().len(), before);
    }

    #[test]
    fn a_journal_title_cannot_be_edited_through_the_chip_either() {
        let (mut app, _dir) = fresh_app();
        seed(&mut app);
        app.go_today().unwrap();
        let before = app.workspace.log().len();

        let refusal = app.retitle_current_page("Monday").expect_err("refused");

        assert!(refusal.contains("date"), "got {refusal}");
        assert_eq!(app.workspace.log().len(), before);
    }
}
