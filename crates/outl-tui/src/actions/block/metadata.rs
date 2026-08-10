//! Block-level and page-level metadata writes: properties, TODO
//! prefix cycle, the `pinned::` flag.
//!
//! These commit straight to disk through `save()` (or the source-page
//! variant for backlinks) and bypass Insert mode entirely — they're
//! invoked from slash commands, chord shortcuts, or the command
//! palette.

use crate::outline_ops::{node_at_path, node_at_path_mut, path_for_index};
use crate::state::{App, Focus, ToastKind, View};

impl App {
    /// Set (or replace) a property on the currently selected block.
    /// If `value` is empty the property is **removed** — gives users
    /// a single command for both edit and delete.
    ///
    /// Bound to `/prop <key> <value>` and `:prop <key> <value>`. Idempotent.
    ///
    /// Key match is case-insensitive, matching the parser and
    /// [`Self::property_on_current_block`]. Comparing exactly here made
    /// the reader and the writer disagree about the same property: a
    /// hand-typed `Remind:: 3pm` was found on read, so `:prop remind`
    /// reported a delete and left it on the block, and an overwrite
    /// appended a second `remind::` beside it.
    pub(crate) fn set_property_on_current_block(&mut self, key: &str, value: &str) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            self.status = "no block selected".into();
            return;
        };
        self.snapshot_for_undo();
        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
            if value.is_empty() {
                node.properties
                    .retain(|(k, _)| !k.eq_ignore_ascii_case(key));
                self.status = format!("removed property `{key}`");
            } else if let Some(p) = node
                .properties
                .iter_mut()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
            {
                p.1 = value.to_string();
                self.status = format!("set {key} = {value}");
            } else {
                node.properties.push((key.to_string(), value.to_string()));
                self.status = format!("added {key} = {value}");
            }
        }
        self.save();
    }

    /// Read a property off the currently selected block, or `None`.
    ///
    /// The counterpart to [`Self::set_property_on_current_block`], and
    /// it reads the same place that writes: the parsed AST, not the
    /// workspace tree. The two only meet at a save boundary, so asking
    /// the op log reports a value the user cannot see on screen yet.
    ///
    /// Key match is case-insensitive, matching the parser.
    pub(crate) fn property_on_current_block(&self, key: &str) -> Option<String> {
        let path = path_for_index(&self.page.blocks, self.selected)?;
        node_at_path(&self.page.blocks, &path)?
            .properties
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    }

    /// Set (or replace) a *page-level* property — the ones at the
    /// top of the `.md` (`title::`, `icon::`, ...). Empty value
    /// removes. Bound to `/prop-page <key> <value>`.
    pub(crate) fn set_property_on_page(&mut self, key: &str, value: &str) {
        self.snapshot_for_undo();
        if value.is_empty() {
            self.page.properties.retain(|(k, _)| k != key);
            self.status = format!("removed page property `{key}`");
        } else if let Some(p) = self.page.properties.iter_mut().find(|(k, _)| k == key) {
            p.1 = value.to_string();
            self.status = format!("set page {key} = {value}");
        } else {
            self.page
                .properties
                .push((key.to_string(), value.to_string()));
            self.status = format!("added page {key} = {value}");
        }
        self.save();
    }

    /// Toggle the `pinned:: true` page-level property. Wired to the
    /// `gp` chord in Normal mode and to the `/pin` slash command;
    /// commits straight to disk (no insert-mode buffer to worry
    /// about) and toasts the new state so the user can confirm
    /// without reading the file.
    ///
    /// Refuses to act on Journal pages — pinning a journal would be
    /// semantically weird (today's note auto-rotates) and would
    /// silently dilute the sidebar's `Pinned` list with
    /// date-shaped junk.
    pub(crate) fn toggle_pinned(&mut self) {
        if matches!(self.view, View::Journal(_)) {
            self.toast(ToastKind::Warning, "can't pin a journal page");
            return;
        }
        self.snapshot_for_undo();
        let was_pinned = self.page.properties.iter().any(|(k, v)| {
            k == "pinned"
                && matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "true" | "yes" | "1" | "on"
                )
        });
        if was_pinned {
            self.page.properties.retain(|(k, _)| k != "pinned");
            self.save();
            self.toast(ToastKind::Info, "unpinned");
        } else {
            // Drop any existing falsy `pinned::` value first so the
            // toggle doesn't leave two `pinned::` lines stacked at
            // the top of the file.
            self.page.properties.retain(|(k, _)| k != "pinned");
            self.page
                .properties
                .push(("pinned".to_string(), "true".to_string()));
            self.save();
            self.toast(ToastKind::Success, "pinned");
        }
    }

    /// Cycle the focused block's TODO state: none → `TODO ` → `DONE ` →
    /// none. Dispatches by `Focus`: outline blocks edit `app.page`
    /// directly; backlink blocks route through
    /// [`Self::toggle_todo_backlink`] which loads the source page off
    /// disk.
    pub(crate) fn toggle_todo(&mut self) {
        match self.focus.clone() {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return;
                };
                self.snapshot_for_undo();
                if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
                    node.text = super::cycle_todo_state(&node.text);
                }
                self.save();
            }
            Focus::Backlink { idx, sub_path } => {
                self.toggle_todo_backlink(idx, &sub_path);
            }
        }
    }
}

#[cfg(test)]
mod property_edit_tests {
    //! `:prop <key> <value>` is the TUI's property editor, and the
    //! `remind::` rule is the property most likely to be edited after
    //! it's written (`g r` seeds a starter the user then tunes).
    //!
    //! These drive the real command through a real `App`. An earlier
    //! version asserted on `args.split_once(' ')` inline, which passed
    //! whatever the command did and caught nothing.

    use crate::commands::CommandRegistry;
    use crate::state::App;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let app = App::new(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    fn seed_single_block(app: &mut App, text: &str) {
        app.page.blocks.clear();
        app.page.blocks.push(outl_md::parse::OutlineNode {
            text: text.to_string(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
    }

    /// Runs `:prop <args>` exactly as the command palette does, name
    /// resolution and arg splitting included.
    fn run_prop(app: &mut App, args: &str) {
        CommandRegistry::with_builtins()
            .dispatch(app, &format!("prop {args}"))
            .unwrap();
    }

    #[test]
    fn a_rule_with_spaces_is_not_truncated_at_the_first_word() {
        // The whole grammar is the value. Splitting on every space
        // would store `3pm` and drop the repeat without saying so.
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "ship it");

        run_prop(&mut app, "remind 3pm every 1h until DONE");

        assert_eq!(
            app.property_on_current_block("remind").as_deref(),
            Some("3pm every 1h until DONE")
        );
    }

    #[test]
    fn an_empty_value_is_the_delete_path() {
        // How a user stops a block nagging without deleting the block.
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "ship it");
        run_prop(&mut app, "remind 9am");

        run_prop(&mut app, "remind");

        assert_eq!(app.property_on_current_block("remind"), None);
        assert!(
            app.page.blocks[0].properties.is_empty(),
            "the pair has to leave the AST, not just stop being found"
        );
    }

    #[test]
    fn a_differently_cased_key_is_replaced_not_duplicated() {
        // `Remind::` parses and fires like `remind::`, so the editor
        // has to treat them as one property. Comparing exactly here
        // appended a second pair and the block ended up with two
        // rules, only one of which the user could see they'd written.
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "ship it");
        app.page.blocks[0]
            .properties
            .push(("Remind".to_string(), "9am".to_string()));

        run_prop(&mut app, "remind 3pm");

        assert_eq!(
            app.page.blocks[0].properties.len(),
            1,
            "expected the existing pair to be replaced, got {:?}",
            app.page.blocks[0].properties
        );
        assert_eq!(
            app.property_on_current_block("REMIND").as_deref(),
            Some("3pm")
        );
    }

    #[test]
    fn a_differently_cased_key_is_deleted_too() {
        // The other half of the same bug: the delete reported success
        // and left `Remind:: 9am` on the block, still firing.
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "ship it");
        app.page.blocks[0]
            .properties
            .push(("Remind".to_string(), "9am".to_string()));

        run_prop(&mut app, "remind");

        assert!(
            app.page.blocks[0].properties.is_empty(),
            "expected the delete to reach it, got {:?}",
            app.page.blocks[0].properties
        );
    }
}
