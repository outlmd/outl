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

    /// Cycle the focused block's task state: none → `TODO ` →
    /// `DOING ` → `DONE ` → none.
    /// Dispatches by `Focus`: outline blocks edit `app.page`
    /// directly; backlink blocks route through
    /// [`Self::toggle_todo_backlink`] which loads the source page off
    /// disk.
    ///
    /// A query-result row is a special outline block: its whole text is a
    /// single `!((blk-…))` embed token. Cycling it in place would prefix
    /// the token (`TODO !((blk-…))`) and destroy the ref, so when the
    /// selected block *is* an embed we instead cycle the **source** block's
    /// state — the same cross-page path backlinks use.
    pub(crate) fn toggle_todo(&mut self) {
        match self.focus.clone() {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return;
                };
                let embed_handle = node_at_path(&self.page.blocks, &path)
                    .and_then(|node| crate::view::embed_only_handle(&node.text))
                    .map(str::to_owned);
                if let Some(handle) = embed_handle {
                    self.cycle_embed_source_status(&handle);
                    return;
                }
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

    /// Cycle the task state of the *source* block behind a query-result
    /// embed row. The row itself is one `!((blk-…))` token, so we resolve
    /// the handle to its origin page, load that page, flip the referenced
    /// block's prefix, and save through [`Self::save_page_with`] so
    /// reconcile keeps IDs stable — mirroring [`Self::toggle_todo_backlink`].
    ///
    /// After the source page lands we re-run the current page's auto-run
    /// blocks (the query fence) against the freshly-patched index, so the
    /// result list reflects the new state: a `DONE` item drops off a
    /// `status: todo` query on the spot.
    ///
    /// No undo snapshot — same rationale as the backlink path: undo here
    /// shouldn't silently flip a TODO in a different file.
    fn cycle_embed_source_status(&mut self, handle: &str) {
        // The source block may live on the *current* page, whose in-memory
        // AST can hold a coalesced edit not yet on disk. Flush first so the
        // read below sees post-edit state — otherwise we'd cycle a stale
        // snapshot and the later `persist()` in `load_current_no_autorun`
        // would revert the flip. Same "readers flush first" rule as every
        // other path that reads a `.md` after an edit.
        self.flush_pending_save();
        let (source_path, source_block_path) = match self.index.resolve_block_ref(handle) {
            Some(entry) => (entry.source_path.clone(), entry.source_block_path.clone()),
            None => {
                self.status = format!("orphan ref: {handle}");
                return;
            }
        };
        let text = match outl_md::read_for_rewrite(&source_path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("cannot read source {}: {e}", source_path.display());
                return;
            }
        };
        let mut source_page = outl_md::parse::parse(&text);
        let Some(node) = node_at_path_mut(&mut source_page.blocks, &source_block_path) else {
            self.status = "source block missing — index may be stale".into();
            return;
        };
        let new_text = super::cycle_todo_state(&node.text);
        let (state, body) = outl_actions::todo::split_todo(&new_text);
        let label = match state {
            Some(outl_actions::TodoState::Todo) => "TODO",
            Some(outl_actions::TodoState::Doing) => "DOING",
            Some(outl_actions::TodoState::Done) => "DONE",
            None => "unmarked",
        };
        let status_msg = format!("{label} · {}", body.trim());
        node.text = new_text;
        let saved = self.save_page_with(&source_path, &source_page, true);
        self.run_auto_run_blocks();
        // Surface the new state unless the save failed or the auto-run
        // pass reported an error — each of those messages names what went
        // wrong and must not be buried under a false "it worked" line.
        if saved && !self.status.starts_with("auto-run skipped") {
            self.status = status_msg;
        }
    }

    /// Land the focused block on `DONE` outright, whatever its state was.
    /// Dispatch mirrors [`Self::toggle_todo`] exactly — outline blocks edit
    /// `app.page` directly, backlink blocks route through
    /// [`Self::mark_done_backlink`], and an embed-only row resolves to its
    /// source page rather than prefixing the token in place.
    ///
    /// Bound to `g D`. Unlike `Ctrl+T`, this never cycles: pressing it on a
    /// block already `DONE` is a no-op on the content, and pressing it from
    /// any other state (or none) lands `DONE` in one press rather than
    /// walking `TODO → DOING → DONE`.
    pub(crate) fn mark_done(&mut self) {
        match self.focus.clone() {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return;
                };
                let embed_handle = node_at_path(&self.page.blocks, &path)
                    .and_then(|node| crate::view::embed_only_handle(&node.text))
                    .map(str::to_owned);
                if let Some(handle) = embed_handle {
                    self.mark_embed_source_done(&handle);
                    return;
                }
                self.snapshot_for_undo();
                if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
                    node.text = super::done_todo_state(&node.text);
                }
                self.save();
            }
            Focus::Backlink { idx, sub_path } => {
                self.mark_done_backlink(idx, &sub_path);
            }
        }
    }

    /// Land the *source* block behind a query-result embed row on `DONE`.
    /// Same cross-page save path as [`Self::cycle_embed_source_status`], and
    /// the same reasons for every step of it (flush first so a coalesced
    /// edit isn't read from a stale snapshot, re-run auto-run blocks so the
    /// query result reflects the change, no undo snapshot).
    fn mark_embed_source_done(&mut self, handle: &str) {
        self.flush_pending_save();
        let (source_path, source_block_path) = match self.index.resolve_block_ref(handle) {
            Some(entry) => (entry.source_path.clone(), entry.source_block_path.clone()),
            None => {
                self.status = format!("orphan ref: {handle}");
                return;
            }
        };
        let text = match outl_md::read_for_rewrite(&source_path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("cannot read source {}: {e}", source_path.display());
                return;
            }
        };
        let mut source_page = outl_md::parse::parse(&text);
        let Some(node) = node_at_path_mut(&mut source_page.blocks, &source_block_path) else {
            self.status = "source block missing — index may be stale".into();
            return;
        };
        let new_text = super::done_todo_state(&node.text);
        let (_, body) = outl_actions::todo::split_todo(&new_text);
        let status_msg = format!("DONE · {}", body.trim());
        node.text = new_text;
        let saved = self.save_page_with(&source_path, &source_page, true);
        self.run_auto_run_blocks();
        // Same save-failure masking guard as
        // [`Self::cycle_embed_source_status`] — never bury a disk error
        // under a confident "DONE · …".
        if saved && !self.status.starts_with("auto-run skipped") {
            self.status = status_msg;
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

#[cfg(test)]
mod embed_todo_tests {
    //! Ctrl+T on a query-result row. A row whose whole text is one
    //! `!((blk-…))` embed token must cycle the *source* block's state,
    //! not prefix the token in place — prefixing it (`TODO !((blk-…))`)
    //! destroys the reference. These drive the real dispatch through a
    //! real `App` with a real on-disk source page and a live index.

    use crate::state::{App, Focus, View};
    use outl_core::{ActorId, Workspace};
    use outl_md::parse::OutlineNode;
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
            outl_config::TuiIconStyle::Emoji,
        )
        .unwrap();
        (app, dir)
    }

    /// Write a real source page with one block, reconcile it so the
    /// block gets a stable handle, and patch the index. Returns that
    /// handle. Mirrors `save_page_with` minus the backlink rebuild and
    /// peer announce, which are irrelevant here and would spawn a
    /// background thread outliving the temp dir.
    fn seed_source_page(app: &mut App, slug: &str, text: &str) -> String {
        let path = app.workspace_root.join("pages").join(format!("{slug}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let page = outl_md::parse::parse(&format!("- {text}\n"));
        outl_md::write_atomic(&path, outl_md::render::render(&page).as_bytes()).unwrap();
        outl_md::reconcile::reconcile_md(
            &mut app.workspace,
            &app.hlc,
            &path,
            Some(&app.orphans_log),
        )
        .unwrap();
        app.index.patch_page(&path, &page);
        let entries: Vec<_> = app.index.iter_blocks().collect();
        assert_eq!(entries.len(), 1, "exactly one block should be indexed");
        entries[0].ref_handle.clone()
    }

    /// Point the app at a query page whose single selected row is an
    /// embed of `handle`, in outline focus — the shape of a query result.
    fn select_embed_row(app: &mut App, dir: &TempDir, slug: &str, handle: &str) {
        app.view = View::Page(dir.path().join("pages").join(format!("{slug}.md")));
        app.page.blocks.clear();
        app.page.blocks.push(OutlineNode {
            text: format!("!(({handle}))"),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
        app.focus = Focus::Outline;
    }

    #[test]
    fn cycling_an_embed_row_flips_the_source_block_not_the_token() {
        let (mut app, dir) = fresh_app();
        let handle = seed_source_page(&mut app, "groceries", "TODO buy milk");
        select_embed_row(&mut app, &dir, "query", &handle);

        app.toggle_todo();

        // The row stays a pristine embed token — prefixing it would
        // destroy the reference (the original bug).
        assert_eq!(app.page.blocks[0].text, format!("!(({handle}))"));
        // The source block cycled TODO -> DOING on its own page…
        let source = dir.path().join("pages").join("groceries.md");
        let source_text = std::fs::read_to_string(source).unwrap();
        assert!(
            source_text.contains("DOING buy milk"),
            "source should have cycled, got: {source_text}"
        );
        // …and the index reflects the new state under the same handle.
        let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
        assert_eq!(entry.text, "DOING buy milk");
    }

    #[test]
    fn cycling_an_embed_row_twice_reaches_done() {
        let (mut app, dir) = fresh_app();
        let handle = seed_source_page(&mut app, "groceries", "TODO buy milk");
        select_embed_row(&mut app, &dir, "query", &handle);

        app.toggle_todo();
        app.toggle_todo();

        let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
        assert_eq!(entry.text, "DONE buy milk");
    }

    #[test]
    fn an_orphan_embed_handle_is_a_noop() {
        let (mut app, dir) = fresh_app();
        select_embed_row(&mut app, &dir, "query", "blk-deadbe");

        app.toggle_todo();

        // Nothing resolved, nothing cycled, the token is untouched.
        assert_eq!(app.page.blocks[0].text, "!((blk-deadbe))");
        assert!(app.status.contains("orphan"), "got: {}", app.status);
    }

    #[test]
    fn a_plain_block_still_cycles_in_place() {
        let (mut app, _dir) = fresh_app();
        app.page.blocks.clear();
        app.page.blocks.push(OutlineNode {
            text: "buy milk".into(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
        app.focus = Focus::Outline;

        app.toggle_todo();

        assert_eq!(app.page.blocks[0].text, "TODO buy milk");
    }

    #[test]
    fn cycling_a_same_page_source_keeps_unflushed_edits() {
        // Regression: the source block lives on the *current* page, which
        // holds a coalesced (unflushed) edit. Reading the source from disk
        // without flushing first cycles a stale snapshot and clobbers the
        // edit — the flip must land on post-edit state.
        let (mut app, dir) = fresh_app();
        let path = dir.path().join("pages").join("task.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        // On-disk state: just the source block, reconciled for a handle.
        let source_only = outl_md::parse::parse("- TODO buy milk\n");
        outl_md::write_atomic(&path, outl_md::render::render(&source_only).as_bytes()).unwrap();
        outl_md::reconcile::reconcile_md(
            &mut app.workspace,
            &app.hlc,
            &path,
            Some(&app.orphans_log),
        )
        .unwrap();
        app.index.patch_page(&path, &source_only);
        let handle = app.index.iter_blocks().next().unwrap().ref_handle.clone();

        // In-memory current page: the source block, an embed row pointing at
        // it, and a coalesced edit that exists only in memory (not flushed).
        app.view = View::Page(path.clone());
        app.page.blocks.clear();
        app.page.blocks.push(OutlineNode {
            text: "TODO buy milk".into(),
            children: vec![],
            properties: vec![],
        });
        app.page.blocks.push(OutlineNode {
            text: format!("!(({handle}))"),
            children: vec![],
            properties: vec![],
        });
        app.page.blocks.push(OutlineNode {
            text: "unsaved note".into(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 3;
        app.selected = 1; // the embed row
        app.focus = Focus::Outline;
        app.save(); // mark dirty — the coalesced edit is not yet on disk

        app.toggle_todo();

        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            disk.contains("DOING buy milk"),
            "source should have cycled, got: {disk}"
        );
        assert!(
            disk.contains("unsaved note"),
            "coalesced edit must survive the cross-save, got: {disk}"
        );
    }
}

#[cfg(test)]
mod embed_done_tests {
    //! `g D` on a query-result row mirrors [`embed_todo_tests`] exactly
    //! except for the destination: every press lands `DONE`, whatever
    //! state the source was in — not one step closer to it. See
    //! [`App::mark_done`] for why "finish this" and "cycle" are different
    //! asks (a block carrying a rule but no marker would arm `TODO `
    //! under a cycle instead of settling).

    use crate::state::{App, Focus, View};
    use outl_core::{ActorId, Workspace};
    use outl_md::parse::OutlineNode;
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
            outl_config::TuiIconStyle::NerdFont,
        )
        .unwrap();
        (app, dir)
    }

    /// See [`super::embed_todo_tests`] — same fixtures, needed here too
    /// because there's no shared test-helper module to pull from.
    fn seed_source_page(app: &mut App, slug: &str, text: &str) -> String {
        let path = app.workspace_root.join("pages").join(format!("{slug}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let page = outl_md::parse::parse(&format!("- {text}\n"));
        outl_md::write_atomic(&path, outl_md::render::render(&page).as_bytes()).unwrap();
        outl_md::reconcile::reconcile_md(
            &mut app.workspace,
            &app.hlc,
            &path,
            Some(&app.orphans_log),
        )
        .unwrap();
        app.index.patch_page(&path, &page);
        let entries: Vec<_> = app.index.iter_blocks().collect();
        assert_eq!(entries.len(), 1, "exactly one block should be indexed");
        entries[0].ref_handle.clone()
    }

    fn select_embed_row(app: &mut App, dir: &TempDir, slug: &str, handle: &str) {
        app.view = View::Page(dir.path().join("pages").join(format!("{slug}.md")));
        app.page.blocks.clear();
        app.page.blocks.push(OutlineNode {
            text: format!("!(({handle}))"),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
        app.focus = Focus::Outline;
    }

    #[test]
    fn marking_an_embed_row_done_flips_the_source_not_the_token() {
        let (mut app, dir) = fresh_app();
        let handle = seed_source_page(&mut app, "groceries", "buy milk");
        select_embed_row(&mut app, &dir, "query", &handle);

        app.mark_done();

        // The row stays a pristine embed token — prefixing it would
        // destroy the reference, same trap as Ctrl+T.
        assert_eq!(app.page.blocks[0].text, format!("!(({handle}))"));
        let source = dir.path().join("pages").join("groceries.md");
        let source_text = std::fs::read_to_string(source).unwrap();
        assert!(
            source_text.contains("DONE buy milk"),
            "source should have landed DONE from a bare block, got: {source_text}"
        );
        let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
        assert_eq!(entry.text, "DONE buy milk");
    }

    #[test]
    fn marking_an_embed_row_done_lands_from_any_earlier_state_in_one_press() {
        // Where `toggle_todo` needs two presses to walk TODO -> DOING ->
        // DONE, `mark_done` needs one from any of them.
        let (mut app, dir) = fresh_app();
        let handle = seed_source_page(&mut app, "groceries", "TODO buy milk");
        select_embed_row(&mut app, &dir, "query", &handle);

        app.mark_done();

        let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
        assert_eq!(entry.text, "DONE buy milk");
    }

    #[test]
    fn marking_an_embed_row_done_twice_is_idempotent() {
        let (mut app, dir) = fresh_app();
        let handle = seed_source_page(&mut app, "groceries", "DOING buy milk");
        select_embed_row(&mut app, &dir, "query", &handle);

        app.mark_done();
        app.mark_done();

        let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
        assert_eq!(entry.text, "DONE buy milk");
    }

    #[test]
    fn an_orphan_embed_handle_mark_done_is_a_noop() {
        let (mut app, dir) = fresh_app();
        select_embed_row(&mut app, &dir, "query", "blk-deadbe");

        app.mark_done();

        assert_eq!(app.page.blocks[0].text, "!((blk-deadbe))");
        assert!(app.status.contains("orphan"), "got: {}", app.status);
    }

    #[test]
    fn a_plain_block_marks_done_in_place() {
        let (mut app, _dir) = fresh_app();
        app.page.blocks.clear();
        app.page.blocks.push(OutlineNode {
            text: "buy milk".into(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
        app.focus = Focus::Outline;

        app.mark_done();

        assert_eq!(app.page.blocks[0].text, "DONE buy milk");
    }

    #[test]
    fn marking_a_same_page_source_done_keeps_unflushed_edits() {
        // Same regression shape as
        // [`embed_todo_tests::cycling_a_same_page_source_keeps_unflushed_edits`],
        // reached through the other door: `mark_embed_source_done` must
        // flush first too, or the DONE flip cycles off a stale snapshot
        // and clobbers a coalesced edit sitting on the current page.
        let (mut app, dir) = fresh_app();
        let path = dir.path().join("pages").join("task.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let source_only = outl_md::parse::parse("- TODO buy milk\n");
        outl_md::write_atomic(&path, outl_md::render::render(&source_only).as_bytes()).unwrap();
        outl_md::reconcile::reconcile_md(
            &mut app.workspace,
            &app.hlc,
            &path,
            Some(&app.orphans_log),
        )
        .unwrap();
        app.index.patch_page(&path, &source_only);
        let handle = app.index.iter_blocks().next().unwrap().ref_handle.clone();

        app.view = View::Page(path.clone());
        app.page.blocks.clear();
        app.page.blocks.push(OutlineNode {
            text: "TODO buy milk".into(),
            children: vec![],
            properties: vec![],
        });
        app.page.blocks.push(OutlineNode {
            text: format!("!(({handle}))"),
            children: vec![],
            properties: vec![],
        });
        app.page.blocks.push(OutlineNode {
            text: "unsaved note".into(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 3;
        app.selected = 1;
        app.focus = Focus::Outline;
        app.save();

        app.mark_done();

        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            disk.contains("DONE buy milk"),
            "source should have landed DONE, got: {disk}"
        );
        assert!(
            disk.contains("unsaved note"),
            "coalesced edit must survive the cross-save, got: {disk}"
        );
    }
}
