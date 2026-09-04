//! Run the code block under the cursor through [`outl_exec`].
//!
//! The result subblock lives in the `.md` (`> **result:**` child of
//! the code block). We commit any in-flight Insert first so the
//! runtime sees the user's freshest source, then reparse so the
//! in-memory AST picks up the newly inserted/updated result child.

use crate::state::{App, Mode};
use outl_actions::{parse_call_invocation, run_callable_block};
use outl_core::id::NodeId;
use outl_md::parse::OutlineNode;
use std::time::Duration;

impl App {
    /// Check if the block at `idx` is a `call:<template>` fence.
    /// If so, run it through the shared callable-template path and
    /// insert the result as a `> **result:**` subblock. Returns `true`
    /// when handled (caller should skip normal exec).
    fn maybe_run_call_block(&mut self, idx: usize) -> bool {
        let mut cursor = 0usize;
        let block = find_block_at_flat(&self.page.blocks, idx, &mut cursor).cloned();
        let Some(block) = block else {
            return false;
        };
        let Some((name, params)) = parse_call_invocation(&block.text) else {
            return false;
        };
        let anchor = self.id_by_flat.get(idx).copied().unwrap_or(NodeId::root());

        match self.run_callable_template(&name, &params, anchor) {
            Ok(dur) => self.status = format!("ran call:{name} ({}ms)", dur.as_millis()),
            Err(e) => self.status = format!("call: {e}"),
        }
        true
    }

    /// Run callable template `name` with `params`, attaching the result
    /// under `anchor`, then reproject + reload the page.
    ///
    /// Both the `call:<name>` fence (`gx`) and the `/template <name> k=v`
    /// slash command wrap this. The execution itself lives in
    /// [`outl_actions::run_callable_block`] (shared with the desktop);
    /// the TUI only owns the reproject + AST reload afterwards.
    pub(crate) fn run_callable_template(
        &mut self,
        name: &str,
        params: &[(String, String)],
        anchor: NodeId,
    ) -> Result<Duration, String> {
        let out = run_callable_block(
            &mut self.workspace,
            &self.hlc,
            &self.exec_registry,
            name,
            params,
            anchor,
        )
        .map_err(|e| e.to_string())?;

        // Re-render the page projection, then reload the AST. Guarded
        // (root `CLAUDE.md` invariant 8): the template's result is
        // already durably in the op log from `run_callable_block` above,
        // so a refusal here means only the `.md` didn't pick it up — but
        // that has to reach the caller, not vanish, so it propagates
        // like `run_callable_block`'s own error instead of being
        // swallowed. Both callers (`maybe_run_call_block`,
        // `rerun_call_block_at`) already show `Err` on the status line.
        if let Some(root) = self.workspace.root.clone() {
            let slug = self.current_slug();
            if let Some(page_id) = outl_actions::find_by_slug(&self.workspace, &slug) {
                outl_actions::apply_page_md_with_sidecar_guarded(&self.workspace, &root, page_id)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.load_current();
        Ok(out.duration)
    }

    /// Re-run the block at `path` if it is a `call:<name>` fence, so its
    /// `> **result:**` stays fresh after the user edits the params.
    /// A no-op for every other block. Called on Insert commit.
    pub(crate) fn rerun_call_block_at(&mut self, path: &[usize]) {
        let slug = self.current_slug();
        let Some(page_id) = outl_actions::find_by_slug(&self.workspace, &slug) else {
            return;
        };
        let Some(node) =
            crate::actions::paste::resolve_node_id_at_path(&self.workspace, page_id, path)
        else {
            return;
        };
        let Some(text) = self.workspace.block_text(node) else {
            return;
        };
        let Some((name, params)) = outl_actions::parse_call_invocation(&text) else {
            return;
        };
        match self.run_callable_template(&name, &params, node) {
            Ok(dur) => self.status = format!("ran call:{name} ({}ms)", dur.as_millis()),
            Err(e) => self.status = format!("call: {e}"),
        }
    }

    /// Dispatch the `gx` chord: run the fenced code block under the
    /// cursor, or — when the block isn't code — open the markdown link
    /// `[text](url)` under the cursor in the system browser (issue #183).
    ///
    /// Code execution wins: a fenced block always runs, and only a
    /// non-fence block consults the link under the caret. Neither present
    /// is a friendly status message, not the old `run failed` modal.
    pub(crate) fn run_current_block(&mut self) {
        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }

        match decide_gx(&self.current_block_text(), self.cursor_col) {
            GxAction::OpenLink(url) => {
                self.open_external_url(&url);
                return;
            }
            GxAction::Nothing => {
                self.status = "no code block or link under cursor".into();
                return;
            }
            GxAction::Run => {}
        }

        let path = self.current_path();
        let idx = self.selected;
        let orphans = self.orphans_log.clone();

        // Execution reads the code block from the workspace / `.md`, so
        // persist any coalesced edit first — otherwise `gx` would run the
        // stale source.
        self.flush_pending_save();

        // Skip auto-run runtimes on manual `gx` — they execute
        // automatically on page load and after every save. Running
        // them manually provides no additional value.
        let auto_run_langs = self.collect_auto_run_langs();
        if self.block_flat_is_auto_run_lang(idx, &auto_run_langs) {
            self.status = "query blocks auto-run — no manual execution needed".into();
            return;
        }

        // Intercept `call:<template>` blocks — resolve the template,
        // inject params from the YAML-ish body, execute via the
        // template's runtime, and insert the result.
        if self.maybe_run_call_block(idx) {
            return;
        }

        match outl_exec::run_block_at_index(
            &mut self.workspace,
            &self.hlc,
            &path,
            idx,
            &self.exec_registry,
            Some(&orphans),
            Some(&self.index),
        ) {
            Ok(report) => {
                match &report.result {
                    Ok(out) => {
                        self.status =
                            format!("ran {} in {}ms", report.language, out.duration.as_millis());
                    }
                    Err(e) => {
                        let title = format!("{} runtime error", report.language);
                        self.show_error(title, format!("{e}"));
                    }
                }
                self.load_current();
            }
            Err(e) => {
                self.show_error("run failed", format!("{e}"));
            }
        }
    }

    /// Open a markdown link's target in the system's default handler.
    ///
    /// Two kinds of target:
    /// - A **workspace asset** (`assets/<hash>.<ext>`) resolves to a file
    ///   inside `<workspace>/assets/` via
    ///   [`outl_actions::resolve_asset_path`] (which rejects traversal /
    ///   external schemes) and opens it in the OS default app — a PDF in
    ///   Preview, an image in the viewer. outl renders nothing itself.
    /// - An **external URL** is scheme-guarded to `http` / `https` /
    ///   `mailto`, mirroring the desktop frontend's `openExternalUrl`
    ///   (`outl-frontend-shared/src/api/commands.ts`): `file:`,
    ///   `javascript:`, and anything else are refused so a crafted link
    ///   in a synced note can't launch an arbitrary handler.
    pub(crate) fn open_external_url(&mut self, url: &str) {
        if outl_md::is_asset_link(url) {
            self.open_asset(url);
            return;
        }
        if !is_safe_external_url(url) {
            self.status = format!("refused to open non-web link: {url}");
            return;
        }
        match open::that(url) {
            Ok(()) => self.status = format!("opened {url}"),
            Err(e) => self.show_error("open failed", format!("{e}")),
        }
    }

    /// Resolve a `[name](assets/…)` link against the workspace and open
    /// the file outside outl. A link that resolves to no file on this
    /// device (the asset hasn't synced yet) surfaces a friendly status
    /// line rather than an error modal.
    fn open_asset(&mut self, url: &str) {
        match outl_actions::resolve_asset_path(&self.workspace_root, url) {
            Ok(Some(path)) => match open::that(&path) {
                Ok(()) => self.status = format!("opened {}", path.display()),
                Err(e) => self.show_error("open failed", format!("{e}")),
            },
            Ok(None) => {
                self.status = format!("asset not found on this device yet: {url}");
            }
            Err(e) => self.status = format!("refused to open asset: {e}"),
        }
    }

    /// Run every block on the current page that either:
    /// - carries an `auto-run::` property (cache-aware), or
    /// - uses a runtime whose `auto_run()` returns `true` (always
    ///   re-runs — results depend on workspace state, not the fence
    ///   body).
    ///
    /// Called after each `load_current` and after each `save()`.
    pub(crate) fn run_auto_run_blocks(&mut self) {
        let auto_run_langs = self.collect_auto_run_langs();
        let mut targets: Vec<usize> = Vec::new();
        let mut cursor = 0usize;
        collect_auto_run_targets(
            &self.page.blocks,
            &auto_run_langs,
            &mut cursor,
            &mut targets,
        );
        if targets.is_empty() {
            return;
        }

        let path = self.current_path();
        let orphans = self.orphans_log.clone();
        let mut ran = 0usize;

        for idx in targets {
            // For runtimes with auto_run() == true, bypass the
            // source-hash cache: query results depend on workspace
            // state, not the fence body. For blocks with just the
            // auto-run:: property (no auto_run runtime), keep the
            // cache so navigation is cheap.
            let force = self.block_flat_is_auto_run_lang(idx, &auto_run_langs);
            // Hand the runtime the index this app already maintains
            // (rebuilt on a background thread after each save). The
            // `query` runtime otherwise builds a fresh one off disk per
            // fence — a full walkdir + parse + sidecar read of the
            // workspace, repeated for every query block on the page,
            // on every page load, since `query` auto-runs.
            let result = if force {
                outl_exec::run_block_at_index(
                    &mut self.workspace,
                    &self.hlc,
                    &path,
                    idx,
                    &self.exec_registry,
                    Some(&orphans),
                    Some(&self.index),
                )
                .map(Some)
            } else {
                outl_exec::run_block_at_index_if_source_changed(
                    &mut self.workspace,
                    &self.hlc,
                    &path,
                    idx,
                    &self.exec_registry,
                    Some(&orphans),
                    Some(&self.index),
                )
            };
            match result {
                Ok(Some(_report)) => ran += 1,
                Ok(None) => {}
                Err(e) => {
                    self.status = format!("auto-run skipped block {idx}: {e}");
                }
            }
        }

        if ran > 0 {
            self.load_current_no_autorun();
            self.status = format!("auto-ran {ran} block{}", if ran == 1 { "" } else { "s" });
        }
    }

    /// Build the set of fence languages whose runtime declares
    /// `auto_run() == true`.
    fn collect_auto_run_langs(&self) -> Vec<String> {
        self.exec_registry
            .languages()
            .filter(|lang| {
                self.exec_registry
                    .get(lang)
                    .map(|r| r.auto_run())
                    .unwrap_or(false)
            })
            .map(String::from)
            .collect()
    }

    /// Check whether the block at `flat_idx` uses a fence language
    /// whose runtime has `auto_run() == true`.
    fn block_flat_is_auto_run_lang(&self, flat_idx: usize, langs: &[String]) -> bool {
        let mut cursor = 0usize;
        let block = find_block_at_flat(&self.page.blocks, flat_idx, &mut cursor);
        let Some(b) = block else {
            return false;
        };
        let Some(parts) = outl_exec::extract_fence(&b.text) else {
            return false;
        };
        let canonical = outl_md::lang::canonical(&parts.language).unwrap_or(&parts.language);
        langs.iter().any(|l| l == canonical)
    }
}

/// DFS-preorder walk collecting flat indices of blocks that should
/// auto-run: either they carry the `auto-run::` property, or their
/// fence language is in `auto_run_langs`.
fn collect_auto_run_targets(
    blocks: &[OutlineNode],
    auto_run_langs: &[String],
    cursor: &mut usize,
    out: &mut Vec<usize>,
) {
    for b in blocks {
        let has_prop = b.properties.iter().any(|(k, _)| k == "auto-run");
        let is_auto_lang = if let Some(parts) = outl_exec::extract_fence(&b.text) {
            let canonical = outl_md::lang::canonical(&parts.language).unwrap_or(&parts.language);
            auto_run_langs.iter().any(|l| l == canonical)
        } else {
            false
        };
        if has_prop || is_auto_lang {
            out.push(*cursor);
        }
        *cursor += 1;
        collect_auto_run_targets(&b.children, auto_run_langs, cursor, out);
    }
}

/// Find the block at `target_idx` in DFS preorder. Returns `None` if
/// out of range.
fn find_block_at_flat<'a>(
    blocks: &'a [OutlineNode],
    target_idx: usize,
    cursor: &mut usize,
) -> Option<&'a OutlineNode> {
    for b in blocks {
        if *cursor == target_idx {
            return Some(b);
        }
        *cursor += 1;
        if let Some(found) = find_block_at_flat(&b.children, target_idx, cursor) {
            return Some(found);
        }
    }
    None
}

/// What the `gx` chord should do for the block under the cursor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GxAction {
    /// The block is a fenced code block — run it (code exec has priority).
    Run,
    /// The block isn't code but a markdown link sits under the cursor.
    OpenLink(String),
    /// Neither a code block nor a link under the cursor.
    Nothing,
}

/// Decide what `gx` does given the block `text` and the cursor column.
///
/// Pure so the priority rule (code beats link) is unit-testable without
/// touching the terminal or the browser.
pub(crate) fn decide_gx(text: &str, cursor_col: usize) -> GxAction {
    if outl_exec::extract_fence(text).is_some() {
        GxAction::Run
    } else if let Some(url) = outl_md::link_at_cursor(text, cursor_col) {
        GxAction::OpenLink(url.to_string())
    } else {
        GxAction::Nothing
    }
}

/// Only web-ish schemes may be opened externally: `http`, `https`,
/// `mailto`. Mirrors the desktop frontend's `openExternalUrl` guard.
fn is_safe_external_url(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| {
        matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "mailto"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_block_runs_even_with_a_link_in_it() {
        // A fence wins over any link text inside it.
        let text = "```sh\ncurl https://example.com\n```";
        assert_eq!(decide_gx(text, 10), GxAction::Run);
    }

    #[test]
    fn non_code_block_opens_link_under_cursor() {
        let text = "see [docs](https://outl.app) for more";
        // Cursor on the anchor text.
        assert_eq!(
            decide_gx(text, 6),
            GxAction::OpenLink("https://outl.app".into())
        );
        // Cursor on the URL portion.
        assert_eq!(
            decide_gx(text, 15),
            GxAction::OpenLink("https://outl.app".into())
        );
    }

    #[test]
    fn plain_block_with_no_link_does_nothing() {
        assert_eq!(decide_gx("just some prose", 3), GxAction::Nothing);
    }

    #[test]
    fn cursor_off_the_link_does_nothing() {
        let text = "tail [x](https://y.z)";
        assert_eq!(decide_gx(text, 1), GxAction::Nothing);
    }

    #[test]
    fn scheme_guard_allows_web_and_mail_only() {
        assert!(is_safe_external_url("https://outl.app"));
        assert!(is_safe_external_url("http://outl.app"));
        assert!(is_safe_external_url("HTTPS://OUTL.APP"));
        assert!(is_safe_external_url("mailto:a@b.c"));
        assert!(!is_safe_external_url("file:///etc/passwd"));
        assert!(!is_safe_external_url("javascript:alert(1)"));
        assert!(!is_safe_external_url("outl.app"));
    }

    /// Unlike this crate's `open_in_memory` test convention, this test
    /// needs the real disk-root path: `run_callable_template`'s
    /// reprojection guard is gated on `self.workspace.root.is_some()`,
    /// exactly like production (`runtime.rs` always opens with
    /// `Some(root)`). See `template.rs`'s identical helper for the
    /// same reasoning.
    fn test_app_with_root() -> (crate::state::App, tempfile::TempDir) {
        use outl_core::id::ActorId;
        use outl_core::storage::JsonlStorage;
        use outl_core::workspace::Workspace;

        let dir = tempfile::TempDir::new().unwrap();
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

    /// The site this test guards: `run_callable_template` used to
    /// reproject today's journal through the unconditional writer right
    /// after the callable template's blocks landed in the op log, so a
    /// frozen page got flattened by a successful `call:`/`/template`
    /// run. Root `CLAUDE.md` invariant 8.
    #[test]
    fn run_callable_template_refuses_to_reproject_a_frozen_page() {
        let (mut app, _dir) = test_app_with_root();

        // Give today's journal one real block so its sidecar can
        // "answer" (see `sidecar_can_answer` — an all-empty-text
        // sidecar is treated as pre-0.11 and always let through).
        let slug = app.current_slug();
        let page_id = outl_actions::find_by_slug(&app.workspace, &slug).unwrap();
        let anchor =
            outl_actions::append_block(&mut app.workspace, &app.hlc, Some(page_id), Some("first"))
                .unwrap();
        outl_actions::apply_page_md_with_sidecar(&app.workspace, &app.workspace_root, page_id)
            .unwrap();
        app.load_current_no_autorun();

        // Callable template using the built-in test-only `echo` runtime
        // (always registered under `cfg(any(test, debug_assertions))`).
        let tpl = outl_actions::open_or_create_page(
            &mut app.workspace,
            &app.hlc,
            "template-echoer",
            "echoer",
            outl_actions::PageKind::Page,
        )
        .unwrap();
        outl_actions::set_property(
            &mut app.workspace,
            &app.hlc,
            tpl,
            outl_actions::TEMPLATE_KEY,
            Some(outl_core::property::PropValue::Text("echoer".into())),
        )
        .unwrap();
        outl_actions::append_block(
            &mut app.workspace,
            &app.hlc,
            Some(tpl),
            Some("```echo\nhi\n```"),
        )
        .unwrap();

        // Freeze today's journal `.md`: content the op log has never
        // seen, sidecar re-stamped to call those exact bytes faithful.
        let md_path = app.current_path();
        let mut md = std::fs::read_to_string(&md_path).unwrap();
        md.push_str("- only ever on disk\n");
        std::fs::write(&md_path, &md).unwrap();
        let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
        let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
        sc.last_synced_hash = outl_md::sidecar::file_hash(&md);
        outl_md::sidecar::write(&sidecar_path, &sc).unwrap();

        let result = app.run_callable_template("echoer", &[], anchor);

        match result {
            Err(e) => assert!(
                e.contains("only ever on disk") || e.contains("no op"),
                "error must surface the refusal, got: {e}"
            ),
            Ok(dur) => panic!("expected a refusal, got Ok({dur:?})"),
        }
        let after = std::fs::read_to_string(&md_path).unwrap();
        assert!(
            after.contains("only ever on disk"),
            "a refused reprojection must never delete the unlogged content: {after:?}"
        );
    }
}
