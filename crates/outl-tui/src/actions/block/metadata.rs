//! Block-level and page-level metadata writes: properties, TODO
//! prefix cycle, the `pinned::` flag.
//!
//! These bypass Insert mode entirely — they're invoked from slash
//! commands, chord shortcuts, or the command palette. A write to the
//! page on screen commits straight to disk through `save()`; a write to
//! a **backlink row** is one op on the citing block's id, through
//! [`App::commit_backlink_mutation`].
//!
//! The two embed-source helpers below are the exception, and still
//! render the source page's `.md`. That is **scope, not a blocker**:
//! they are about embeds rather than backlinks, so they were left out
//! of the move onto the op log. `BlockEntry` does carry the `id` and
//! the `source_slug` a commit would need.

use super::backlink_gesture::BacklinkGesture;
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
    /// Dispatches by `Focus`: outline blocks edit `app.page` directly;
    /// a backlink row is one [`outl_actions::toggle_todo`] on the citing
    /// block's id, through [`Self::commit_backlink_mutation`].
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
            Focus::Backlink { .. } => {
                if let Some(target) = self.focused_backlink_target_or_report() {
                    self.commit_backlink_mutation(
                        target,
                        BacklinkGesture::MarkerFromLoggedText,
                        outl_actions::toggle_todo,
                    );
                }
            }
        }
    }

    /// Cycle the task state of the *source* block behind a query-result
    /// embed row. The row itself is one `!((blk-…))` token, so we resolve
    /// the handle to its origin page, load that page, flip the referenced
    /// block's prefix, and save through [`Self::save_page_with`] so
    /// reconcile keeps IDs stable.
    ///
    /// **This is the `.md`-first route, still, and that is scope
    /// rather than a blocker.** The backlink chords moved onto the op
    /// log and left this one behind deliberately: it is about embeds,
    /// not backlinks.
    ///
    /// Nothing stops it following. `WorkspaceIndex::resolve_block_ref`
    /// hands back a `BlockEntry`, which carries `id: NodeId` and
    /// `source_slug` — both keys [`Self::commit_backlink_mutation`]
    /// needs — and the destructuring below throws `entry.id` away. A
    /// stale id is already its `NotInTree` arm. What the migration owes
    /// is this function's *other* half: the auto-run re-run and the
    /// same-page reload, neither of which the backlink path has.
    ///
    /// After the source page lands we re-run the current page's auto-run
    /// blocks (the query fence) against the freshly-patched index, so the
    /// result list reflects the new state: a `DONE` item drops off a
    /// `status: todo` query on the spot.
    ///
    /// No undo snapshot.
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
        // A same-page source leaves `self.page` on the pre-flip AST. A
        // page with an auto-run fence reloads inside
        // `run_auto_run_blocks`; a hand-written embed row has none, so
        // nothing reloads and the next `persist()` renders the stale AST
        // back over the flip. Reload from disk (which now carries it)
        // whenever the source is the page on screen.
        if saved && source_path == self.current_path() {
            self.load_current_no_autorun();
        }
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
    /// `app.page` directly, a backlink row is one
    /// [`outl_actions::mark_done`] on the citing block's id, and an
    /// embed-only row resolves to its source page rather than prefixing
    /// the token in place.
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
            Focus::Backlink { .. } => {
                if let Some(target) = self.focused_backlink_target_or_report() {
                    self.commit_backlink_mutation(
                        target,
                        BacklinkGesture::MarkerFromLoggedText,
                        outl_actions::mark_done,
                    );
                }
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
        // Same-page reload as [`Self::cycle_embed_source_status`] — a
        // hand-written embed row has no auto-run fence to reload the
        // view, and `self.page` would render its pre-flip AST back.
        if saved && source_path == self.current_path() {
            self.load_current_no_autorun();
        }
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
mod backlink_task_tests {
    //! `Ctrl+T` and `g D` on a backlink row write to a page the user is
    //! not looking at. They used to do it by rendering that page's whole
    //! `.md` and letting `reconcile_md` infer the ops; the op log is the
    //! source of truth, so the op comes first and the `.md` is
    //! downstream of it.
    //!
    //! These drive the real dispatch (`App::toggle_todo` /
    //! `App::mark_done`) over a real two-page workspace and a real
    //! backlink index, because the arm under test resolves its target
    //! through `backlinks_for_current`.

    use crate::actions::block::test_support::workspace_with_a_backlink;

    #[test]
    fn toggling_a_backlinks_task_lands_as_an_op() {
        let (mut app, _d, _source) = workspace_with_a_backlink();
        let target = app
            .focused_backlink_target()
            .expect("a healthy backlink row resolves");

        app.toggle_todo();

        assert_eq!(
            app.workspace.block_text(target.block).as_deref(),
            Some("TODO mentions [[target]]"),
            "the cycle never reached the op log on the citing block"
        );
    }

    /// The citing block's text lives in the op log, so cycling it does
    /// not need the source page's projection to be on this disk.
    ///
    /// The `.md`-first path could not do it at all: it read the file
    /// first and reported `cannot read backlink source` when that
    /// failed. A page a peer has shipped ops for but whose `.md` has
    /// not arrived is exactly that state.
    #[test]
    fn toggling_a_backlinks_task_needs_no_md_on_this_disk() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        let target = app
            .focused_backlink_target()
            .expect("a healthy backlink row resolves");
        std::fs::remove_file(&source).expect("drop the source page's projection");

        app.toggle_todo();

        assert_eq!(
            app.workspace.block_text(target.block).as_deref(),
            Some("TODO mentions [[target]]"),
            "the cycle went looking for a `.md` instead of the op log"
        );
    }

    /// `mark_done` is **not** `toggle_todo`. The citing block carries no
    /// marker, so one cycle would arm `TODO` — which starts the task
    /// instead of closing it.
    #[test]
    fn marking_a_backlink_done_lands_on_done_outright() {
        let (mut app, _d, _source) = workspace_with_a_backlink();
        let target = app
            .focused_backlink_target()
            .expect("a healthy backlink row resolves");

        app.mark_done();

        assert_eq!(
            app.workspace.block_text(target.block).as_deref(),
            Some("DONE mentions [[target]]"),
            "`g D` has to settle the task in one press, from any state"
        );
    }

    /// [#431](https://github.com/outlmd/outl/issues/431): a cross-page
    /// op snapshots the page it changes, under that page's own path, so
    /// `top_applies_here` only offers it once the user opens that page.
    ///
    /// The `.md`-first `g D` recorded nothing at all, on the reasoning
    /// that "undo here shouldn't silently flip a TODO in a different
    /// file" — true while a popped snapshot was applied blind, and an
    /// exemption that outlived its premise once #430 landed.
    #[test]
    fn marking_a_backlink_done_leaves_an_undo_entry_for_the_source_page() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        app.undo.clear();

        app.mark_done();

        let snap = app
            .undo
            .iter()
            .find(|s| s.view_path == source)
            .unwrap_or_else(|| {
                panic!(
                    "no undo entry for {}; stack holds {:?}",
                    source.display(),
                    app.undo.iter().map(|s| &s.view_path).collect::<Vec<_>>()
                )
            });
        assert!(
            snap.page
                .blocks
                .iter()
                .any(|b| b.text == "mentions [[target]]"),
            "the snapshot must carry the pre-mark text, got {:?}",
            snap.page.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod tests;
