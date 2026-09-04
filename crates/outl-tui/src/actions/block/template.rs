use chrono::NaiveDate;

use outl_actions::{find_by_slug, instantiate_template};
use outl_core::id::NodeId;

use crate::actions::paste::resolve_node_id_at_path;
use crate::state::{App, EditTarget, Mode};

impl App {
    /// Instantiate a structural template under the currently
    /// selected block, following the same commit → resolve → apply
    /// → reload pattern as `graft_paste`.
    pub(crate) fn instantiate_template_at_cursor(&mut self, name: &str) {
        let commit_will_save_current = match &self.mode {
            Mode::Insert {
                target,
                buffer,
                original_text,
                ..
            } => matches!(target, EditTarget::CurrentPage) && buffer.as_string() != *original_text,
            _ => false,
        };
        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }
        if !commit_will_save_current {
            self.save();
        }

        let slug = self.current_slug();
        let Some(path) = outl_md::outline_ops::path_for_index(&self.page.blocks, self.selected)
        else {
            self.status = "template: no selected block".into();
            return;
        };

        let Some(page_id) = find_by_slug(&self.workspace, &slug) else {
            self.status = "template: current page not in workspace".into();
            return;
        };

        let Some(target_id) = resolve_node_id_at_path(&self.workspace, page_id, &path) else {
            self.status = "template: could not resolve selected block".into();
            return;
        };

        let page_date = NaiveDate::parse_from_str(&slug, "%Y-%m-%d").ok();

        match instantiate_template(
            &mut self.workspace,
            &self.hlc,
            name,
            target_id,
            &slug,
            page_date,
        ) {
            Ok(ids) => {
                let count = ids.len();

                // Guarded (root `CLAUDE.md` invariant 8): the template's
                // blocks are already in the op log regardless of what this
                // write does, so a refusal here means the page's `.md`
                // holds content no op has seen — say so instead of the
                // ordinary success message papering over a page that just
                // stopped syncing.
                let projection_warning = self.workspace.root.as_ref().and_then(|root| {
                    outl_actions::apply_page_md_with_sidecar_guarded(&self.workspace, root, page_id)
                        .err()
                });

                self.reload_workspace_from_disk();
                self.refresh_page_list();
                self.spawn_index_rebuild();
                self.flat_len = outl_md::outline_ops::flat_count(&self.page.blocks);
                self.pending_chord = None;
                let s = if count == 1 { "" } else { "s" };
                self.status = match projection_warning {
                    Some(e) => {
                        format!("instantiated template `{name}` ({count} block{s}), but {e}")
                    }
                    None => format!("instantiated template `{name}` ({count} block{s})"),
                };
            }
            Err(e) => {
                self.status = format!("template failed: {e}");
            }
        }
    }

    /// Resolve a callable template, execute its code block with the
    /// given params, and attach stdout as a `> **result:**` subtree
    /// under the selected block. Thin wrapper over the shared
    /// [`App::run_callable_template`] so the `/template <name> k=v`
    /// slash command and the `call:` fence (`gx`) stay identical.
    pub(crate) fn execute_callable_template(&mut self, name: &str, params: &[(String, String)]) {
        let anchor = self
            .id_by_flat
            .get(self.selected)
            .copied()
            .unwrap_or(NodeId::root());
        match self.run_callable_template(name, params, anchor) {
            Ok(dur) => self.status = format!("ran template `{name}` ({}ms)", dur.as_millis()),
            Err(e) => self.status = format!("template: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use outl_core::id::ActorId;
    use outl_core::property::PropValue;
    use outl_core::storage::JsonlStorage;
    use outl_core::workspace::Workspace;
    use outl_md::sidecar;
    use tempfile::TempDir;

    /// Unlike the crate-wide `test_app()` convention (`open_in_memory`,
    /// `Workspace::root == None`), this test needs the real disk-root
    /// code path: `instantiate_template_at_cursor`'s reprojection guard
    /// is gated on `self.workspace.root.is_some()`, exactly like
    /// production (`runtime.rs` always opens with `Some(root)`). A
    /// `None` root would make the guarded write a silent no-op and the
    /// test would pass for the wrong reason.
    fn test_app_with_root() -> (crate::state::App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ops_dir = dir.path().join("ops");
        let storage = JsonlStorage::open(ops_dir, actor).unwrap();
        let ws =
            Workspace::open_with_storage(actor, Box::new(storage), Some(dir.path().to_path_buf()))
                .unwrap();
        let app = crate::state::App::new(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    /// The site this test guards: the `Ok(ids)` branch used to reproject
    /// today's journal through the unconditional writer right after the
    /// template's blocks landed in the op log, so a frozen page got
    /// flattened by a successful `/template` run. Root `CLAUDE.md`
    /// invariant 8.
    #[test]
    fn instantiate_template_refuses_to_reproject_a_frozen_page() {
        let (mut app, _dir) = test_app_with_root();

        // Give today's journal one real block so its sidecar can
        // "answer" — an all-empty-text sidecar (the state right after
        // `App::new`'s single seed bullet) is treated as pre-0.11 and
        // the guard always lets it through. See `sidecar_can_answer`.
        let slug = app.current_slug();
        let page_id = outl_actions::find_by_slug(&app.workspace, &slug).unwrap();
        outl_actions::append_block(&mut app.workspace, &app.hlc, Some(page_id), Some("first"))
            .unwrap();
        outl_actions::apply_page_md_with_sidecar(&app.workspace, &app.workspace_root, page_id)
            .unwrap();
        app.load_current_no_autorun();

        // Structural template.
        let tpl = outl_actions::open_or_create_page(
            &mut app.workspace,
            &app.hlc,
            "template-struct",
            "struct",
            outl_actions::PageKind::Page,
        )
        .unwrap();
        outl_actions::set_property(
            &mut app.workspace,
            &app.hlc,
            tpl,
            outl_actions::TEMPLATE_KEY,
            Some(PropValue::Text("struct".into())),
        )
        .unwrap();
        outl_actions::append_block(&mut app.workspace, &app.hlc, Some(tpl), Some("seed block"))
            .unwrap();

        // Freeze today's journal `.md`: content the op log has never
        // seen, with the sidecar re-stamped to call those exact bytes
        // faithful — the state a `reconcile_md` that missed invariant 8
        // leaves behind.
        let md_path = app.current_path();
        let mut md = std::fs::read_to_string(&md_path).unwrap();
        md.push_str("- only ever on disk\n");
        std::fs::write(&md_path, &md).unwrap();
        let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
        let mut sc = sidecar::read(&sidecar_path).unwrap();
        sc.last_synced_hash = sidecar::file_hash(&md);
        sidecar::write(&sidecar_path, &sc).unwrap();
        // `instantiate_template_at_cursor` unconditionally marks the
        // page dirty and later flushes it — a byte-identical render
        // of a STALE in-memory AST would otherwise clobber the frozen
        // line before the guard under test ever sees it, and a render
        // that already knew about the line would let `persist`'s
        // `reconcile_md` absorb it into the op log first (short-
        // circuited here since `last_synced_hash` already matches),
        // which would make it no longer "ahead of the log" by the
        // time this test checks. Parsing the exact frozen bytes into
        // `self.page` keeps that flush a true no-op.
        app.page = outl_md::parse::parse(&md);

        app.instantiate_template_at_cursor("struct");

        assert!(
            app.status.contains("no op") || app.status.contains("PAGE_MARKDOWN_AHEAD_OF_LOG"),
            "status must surface the refusal instead of a plain success, got: {}",
            app.status
        );
        let after = std::fs::read_to_string(&md_path).unwrap();
        assert!(
            after.contains("only ever on disk"),
            "a refused reprojection must never delete the unlogged content: {after:?}"
        );
    }
}
