//! JS plugin integration for the TUI.
//!
//! The TUI embeds [`outl_plugins::PluginHost`] like every other client.
//! This module owns the three touch points:
//!
//! - [`App::load_plugins`] — boot: build the host, declare the
//!   capabilities the TUI honors, load every installed plugin off
//!   `<root>/.outl/plugins/`, and mark the host synced so pre-existing
//!   ops don't fire `onOp` hooks on startup.
//! - [`App::run_plugin_command`] — dispatch a plugin-contributed slash
//!   command (selected in the slash menu) through the host, surface its
//!   notifications / errors, and re-project the page if it mutated the
//!   workspace.
//! - [`App::run_plugin_op_hooks`] — the single post-mutation point: run
//!   every plugin's `onOp` hook over the ops applied since the last
//!   sweep, and re-project if a hook mutated the workspace.
//!
//! Plugins are **best-effort**: a host that can't be built, or a plugin
//! that fails to load, never blocks the TUI. The host is `None` only on
//! the rare total-failure path; an empty host (no plugins installed) is
//! still `Some`.

use crate::state::{App, Overlay, PluginSettingsEntry, PluginSettingsState, ToastKind};
use outl_core::id::NodeId;
use outl_md::parse::OutlineNode;
use std::collections::HashMap;

use outl_plugins::settings;
use outl_plugins::{
    load_installed, lockfile_path, plugins_dir, Capability, ClientCapabilities, InstalledPlugins,
    KeyringStore, PluginHost,
};

/// A pending content-transform: `(block id, plugin id, lang, fence body)`.
///
/// Built by [`collect_transform_jobs`] from the parsed AST so the actual
/// `transform_block` call in [`App::recompute_transforms`] borrows
/// nothing from `self.page` (each field is owned).
type TransformJob = (NodeId, String, String, String);

/// Walk the outline AST in DFS preorder — the same order
/// [`App::id_by_flat`] is built in — and collect one [`TransformJob`]
/// per block whose text is a single fenced code block (`` ```<lang> ``
/// … `` ``` ``) whose language a loaded text transformer claims
/// (`langs`: registered-transformer-lang → plugin_id).
///
/// Match precedence: the fence's **raw** language wins (a transformer
/// for a custom language like `mermaid` registers under that exact
/// string), falling back to the canonical form so a transformer
/// registered as `rust` still fires on a `` ```rs `` fence.
///
/// `ids` is the flat NodeId map; a block with no id (sidecar gap) is
/// skipped — without a key there's nowhere to cache the result. The
/// flat cursor advances over every block (collapsed or not) so it stays
/// aligned with `ids`, mirroring the render walk.
fn collect_transform_jobs(
    blocks: &[OutlineNode],
    ids: &[NodeId],
    langs: &HashMap<String, String>,
) -> Vec<TransformJob> {
    let mut jobs = Vec::new();
    let mut cursor = 0usize;
    collect_jobs_rec(blocks, ids, langs, &mut cursor, &mut jobs);
    jobs
}

fn collect_jobs_rec(
    blocks: &[OutlineNode],
    ids: &[NodeId],
    langs: &HashMap<String, String>,
    cursor: &mut usize,
    jobs: &mut Vec<TransformJob>,
) {
    for b in blocks {
        if let Some(id) = ids.get(*cursor).copied() {
            if let Some((lang_raw, body)) = fence_lang_and_body(&b.text) {
                // Prefer an exact match on the fence's raw language
                // (custom langs), then fall back to its canonical alias.
                if let Some((lang, plugin_id)) = resolve_transformer(lang_raw, langs) {
                    jobs.push((id, plugin_id, lang, body));
                }
            }
        }
        *cursor += 1;
        collect_jobs_rec(&b.children, ids, langs, cursor, jobs);
    }
}

/// Resolve a fence's raw language to `(lang to pass to the plugin,
/// plugin_id)` using the registered-transformer map.
///
/// Raw match first so a transformer registered under a custom string
/// (e.g. `mermaid`) wins verbatim; canonical fallback (via
/// [`outl_md::lang::canonical`]) lets a transformer registered as
/// `rust` fire on a `` ```rs `` fence. The returned lang is the one the
/// transformer registered under — the string `transform_block`
/// dispatches on.
fn resolve_transformer(
    lang_raw: &str,
    langs: &HashMap<String, String>,
) -> Option<(String, String)> {
    if let Some(plugin_id) = langs.get(lang_raw) {
        return Some((lang_raw.to_string(), plugin_id.clone()));
    }
    let canon = outl_md::lang::canonical(lang_raw)?;
    let plugin_id = langs.get(canon)?;
    Some((canon.to_string(), plugin_id.clone()))
}

/// If `text` is a single fenced code block and nothing else, return its
/// `(raw lang, body)`. The raw lang is the fence info-string trimmed of
/// surrounding whitespace (`` ```mermaid `` → `mermaid`). The body
/// excludes the opener and closing `` ``` `` and keeps interior
/// newlines verbatim.
///
/// Returns `None` when the text isn't exactly one closed fence (no
/// opener, no closer, or leading/trailing prose) or the info-string is
/// empty — a bare `` ``` `` fence has no language to match a
/// transformer against.
fn fence_lang_and_body(text: &str) -> Option<(&str, String)> {
    let trimmed = text.trim();
    let mut lines = trimmed.lines();
    let first = lines.next()?;
    let lang_raw = first.trim_start().strip_prefix("```")?.trim();
    if lang_raw.is_empty() {
        return None;
    }

    let mut body_lines: Vec<&str> = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim_start().starts_with("```") {
            closed = true;
            break;
        }
        body_lines.push(line);
    }
    if !closed {
        return None;
    }
    Some((lang_raw, body_lines.join("\n")))
}

impl App {
    /// Capabilities the TUI implements today: slash commands, op hooks,
    /// keybindings, and **text** content-transformers. A plugin's
    /// `contributes.keybindings` chords are resolved against live
    /// Normal-mode keystrokes in `input/plugin_chord.rs` (single- and
    /// two-chord sequences), so the `keybinding` capability is honored,
    /// not just recognised.
    ///
    /// `ContentTransformerText` is declared but **not**
    /// `ContentTransformerRich`: `rich` output is HTML for a GUI iframe
    /// and has no meaning in a terminal. The host filters rich
    /// transformers out of `transformers()` for us, so the TUI only ever
    /// sees text ones.
    fn client_capabilities() -> ClientCapabilities {
        [
            Capability::SlashCommand,
            Capability::OpHook,
            Capability::Keybinding,
            Capability::ContentTransformerText,
            // The terminal has no chrome bar, but a toolbar button is still a
            // runnable command — we surface it in the slash menu instead of
            // dropping it. (`ui-render` / `content-transformer:rich` are HTML,
            // so they stay undeclared — nothing to draw in a terminal.)
            Capability::ToolbarButton,
        ]
        .into_iter()
        .collect()
    }

    /// Rebuild [`App::transform_cache`] for the current view.
    ///
    /// For each block whose text is a single fenced code block
    /// (`` ```<lang> `` … `` ``` ``), if a loaded plugin claims `<lang>`
    /// as a **text** transformer, run the fence body through
    /// [`outl_plugins::PluginHost::transform_block`] and stash the
    /// resulting text/markdown under the block's `NodeId`. The render
    /// path then substitutes the cached output for the raw fence on
    /// read-only (non-cursor) blocks.
    ///
    /// Pre-compute, not render-time: `transform_block` is `&mut self`
    /// (runs Boa) and outline render only has `&App`. Called from
    /// `load_current_no_autorun` (every reparse) and after plugin / peer
    /// mutations. Best-effort: a plugin error or decline (`Ok(None)`)
    /// leaves the block to render as a raw fence — never crashes.
    ///
    /// Cheap when there's nothing to do: the cache is cleared and we
    /// return immediately if there's no host or no text transformers, so
    /// no fence is even inspected.
    pub(crate) fn recompute_transforms(&mut self) {
        self.transform_cache.clear();

        // Take the host out so we can call `&mut` methods on it without
        // aliasing `self` (we also read `self.page` / `self.id_by_flat`).
        // Put it back unconditionally.
        let Some(mut host) = self.plugin_host.take() else {
            return;
        };

        // One lang → plugin_id map for the text transformers granted on
        // this client. Built once per pass; empty means no work to do.
        let langs: std::collections::HashMap<String, String> = host
            .transformers()
            .into_iter()
            .filter(|t| t.kind == "text")
            .map(|t| (t.lang, t.plugin_id))
            .collect();

        if !langs.is_empty() {
            let jobs = collect_transform_jobs(&self.page.blocks, &self.id_by_flat, &langs);
            for (id, plugin_id, lang, body) in jobs {
                match host.transform_block(&plugin_id, &lang, &body) {
                    Ok(Some(res)) if res.kind == "text" => {
                        self.transform_cache.insert(id, res.content);
                    }
                    // Declined (`Ok(None)`), a `rich` result we can't use,
                    // or an engine error: leave the raw fence to render.
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("content-transformer for `{lang}` failed: {e}");
                    }
                }
            }
        }

        self.plugin_host = Some(host);
    }

    /// Build the plugin host and load every installed plugin.
    ///
    /// Best-effort end to end: on any unexpected error this leaves
    /// `plugin_host = None` and returns — the TUI keeps running. A
    /// per-plugin load failure is recorded in the [`load_installed`]
    /// report and surfaced as a toast, but the other plugins still
    /// load.
    pub(crate) fn load_plugins(&mut self) {
        let mut host = PluginHost::new(Self::client_capabilities());
        let dir = plugins_dir(&self.workspace_root);
        let report = load_installed(&mut host, &dir);

        // Pre-existing ops in the log must NOT fire `onOp` hooks at
        // startup — only ops the user produces *after* boot should.
        host.mark_synced(&self.workspace);

        for (id, err) in &report.failed {
            tracing::warn!("plugin {id} failed to load: {err}");
        }
        if !report.failed.is_empty() {
            self.toast(
                ToastKind::Warning,
                format!("{} plugin(s) failed to load", report.failed.len()),
            );
        }
        if !report.loaded.is_empty() {
            tracing::info!("loaded {} plugin(s)", report.loaded.len());
        }

        self.plugin_host = Some(host);
    }

    /// Resolve a command **id** (as the slash menu surfaces it) back to
    /// the `(plugin_id, command_id)` pair `run_plugin_command` needs.
    /// Checks contributed commands first, then toolbar buttons (also
    /// runnable on the TUI). `None` when no loaded plugin owns the id.
    pub(crate) fn find_plugin_command(&self, command_id: &str) -> Option<(String, String)> {
        let host = self.plugin_host.as_ref()?;
        host.commands()
            .into_iter()
            .find(|c| c.command_id == command_id)
            .map(|c| (c.plugin_id, c.command_id))
            .or_else(|| {
                host.toolbar_buttons("tui")
                    .into_iter()
                    .find(|t| t.command_id == command_id)
                    .map(|t| (t.plugin_id, t.command_id))
            })
    }

    /// Run a plugin command picked from the slash menu.
    ///
    /// Surfaces the command's `notify` / log output and any errors via
    /// toasts, then re-projects the current page if the command mutated
    /// the workspace (so the new `.md` shows up immediately).
    pub(crate) fn run_plugin_command(&mut self, plugin_id: &str, command_id: &str) {
        // Take the host out so we can hand `&mut self.workspace` to it
        // without aliasing `self`. Put it back unconditionally.
        let Some(mut host) = self.plugin_host.take() else {
            return;
        };
        let result = host.run_command(&mut self.workspace, &self.hlc, plugin_id, command_id);
        self.plugin_host = Some(host);

        match result {
            Ok(run) => {
                let mutated = run.applied > 0;
                self.surface_plugin_run(&run.notifications, &run.errors);
                if mutated {
                    self.reproject_after_plugin();
                }
            }
            Err(e) => {
                self.toast(ToastKind::Error, format!("plugin command failed: {e}"));
            }
        }
    }

    /// The single post-mutation hook: dispatch every op applied since
    /// the last sweep to plugins' `onOp` handlers. Re-projects the page
    /// if a hook mutated the workspace.
    ///
    /// Cheap and idempotent when nothing changed (the host short-circuits
    /// on `last_seen == log.len()`), so the event loop can call it after
    /// every keystroke that might have touched the workspace.
    pub(crate) fn run_plugin_op_hooks(&mut self) {
        if self.plugin_host.is_none() {
            return;
        }
        // Never run hooks mid-Insert: a hook that mutates the workspace
        // would trigger `reproject_after_plugin` → `load_current`, which
        // reparses from disk and would clobber the in-flight Insert
        // buffer. Insert-mode text isn't in the op log until the commit
        // boundary anyway, so there's nothing new to dispatch yet. The
        // hook fires on the next Normal-mode tick after `commit_insert`
        // appends the edit op. (Same policy as the peer-ops poller's
        // `pending_reload` deferral.)
        if matches!(self.mode, crate::state::Mode::Insert { .. }) {
            return;
        }
        let Some(mut host) = self.plugin_host.take() else {
            return;
        };
        let result = host.sync_hooks(&mut self.workspace, &self.hlc);
        self.plugin_host = Some(host);

        match result {
            Ok(run) => {
                let mutated = run.applied > 0;
                self.surface_plugin_run(&run.notifications, &run.errors);
                if mutated {
                    self.reproject_after_plugin();
                }
            }
            Err(e) => {
                tracing::warn!("plugin op-hook sweep failed: {e}");
            }
        }
    }

    /// Show a plugin's `notify` messages and errors as toasts.
    fn surface_plugin_run(&mut self, notifications: &[String], errors: &[String]) {
        for note in notifications {
            self.toast(ToastKind::Info, note.clone());
        }
        for err in errors {
            self.toast(ToastKind::Error, format!("plugin: {err}"));
        }
    }

    /// Re-project `.md` after a plugin mutated the in-memory workspace.
    ///
    /// A plugin's intents land in the op log via `outl-actions` →
    /// `Workspace::apply`, but **don't** touch the `.md` projection on
    /// their own (the action layer never writes files outside
    /// `write_md_atomic`). A plugin can mutate any page — `archive-done`
    /// moves blocks to a *different* page — so we render every page from
    /// the workspace (`apply_all_pages_md`, the "we don't know which
    /// pages moved" path) before re-parsing the current view. Then
    /// `load_current` rebuilds the AST, id map, and collapsed mirror.
    fn reproject_after_plugin(&mut self) {
        if let Err(e) = outl_actions::apply_all_pages_md(&self.workspace, &self.workspace_root) {
            tracing::warn!("re-projecting .md after plugin mutation failed: {e}");
        }
        self.refresh_page_list();
        // A plugin can touch any page, so rebuild the backlink index
        // off-thread rather than incrementally patching one slug.
        self.spawn_backlink_index_rebuild();
        self.load_current();
    }
}

/// Plugin settings overlay: browse installed plugins' config + secret fields
/// and edit them inline. Config writes go to the lockfile (host reloaded next
/// command); secrets go to the OS keychain. All of it is off the plugin host —
/// plain lockfile + keychain I/O via `outl_plugins::settings`.
impl App {
    /// Open the plugin-settings overlay. No-op with a status hint when no
    /// installed plugin exposes a config schema.
    pub(crate) fn open_plugin_settings(&mut self) {
        let entries = self.collect_plugin_settings();
        if entries.is_empty() {
            self.status = "No configurable plugins installed".to_string();
            return;
        }
        let rows: Vec<(usize, usize)> = entries
            .iter()
            .enumerate()
            .flat_map(|(ei, e)| (0..e.fields.len()).map(move |fi| (ei, fi)))
            .collect();
        self.overlay = Some(Overlay::PluginSettings(PluginSettingsState {
            entries,
            rows,
            selected: 0,
            editing: None,
            message: None,
        }));
    }

    /// Describe every installed plugin, keeping only those with fields.
    fn collect_plugin_settings(&self) -> Vec<PluginSettingsEntry> {
        let dir = plugins_dir(&self.workspace_root);
        let lock = InstalledPlugins::load(&lockfile_path(&dir)).unwrap_or_default();
        let store = KeyringStore::new();
        let mut out = Vec::new();
        for id in lock.plugins.keys() {
            if let Ok(fields) = settings::describe(&self.workspace_root, id, &store) {
                if !fields.is_empty() {
                    out.push(PluginSettingsEntry {
                        plugin_id: id.clone(),
                        fields,
                    });
                }
            }
        }
        out
    }

    /// Move the field cursor by `delta` (clamped). Ignored while editing.
    pub(crate) fn plugin_settings_move(&mut self, delta: isize) {
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            if ps.editing.is_some() || ps.rows.is_empty() {
                return;
            }
            let last = ps.rows.len() - 1;
            let next = (ps.selected as isize + delta).clamp(0, last as isize);
            ps.selected = next as usize;
        }
    }

    /// Enter on a field: booleans toggle immediately; everything else starts
    /// an inline edit (config seeded with the current value, secrets blank).
    pub(crate) fn plugin_settings_activate(&mut self) {
        let Some(Overlay::PluginSettings(ref ps)) = self.overlay else {
            return;
        };
        let Some(&(ei, fi)) = ps.rows.get(ps.selected) else {
            return;
        };
        let field = &ps.entries[ei].fields[fi];

        if !field.secret && matches!(field.kind, settings::FieldKind::Boolean) {
            let current = field
                .value
                .as_ref()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            self.plugin_settings_write(ei, fi, if current { "false" } else { "true" });
            return;
        }

        let seed = if field.secret {
            String::new()
        } else {
            field.value.as_ref().map(value_to_input).unwrap_or_default()
        };
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            ps.editing = Some(seed);
            ps.message = None;
        }
    }

    /// Append a char to the edit buffer.
    pub(crate) fn plugin_settings_edit_push(&mut self, c: char) {
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            if let Some(buf) = ps.editing.as_mut() {
                buf.push(c);
            }
        }
    }

    /// Delete the last char of the edit buffer.
    pub(crate) fn plugin_settings_edit_backspace(&mut self) {
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            if let Some(buf) = ps.editing.as_mut() {
                buf.pop();
            }
        }
    }

    /// Cancel the current inline edit without writing.
    pub(crate) fn plugin_settings_cancel_edit(&mut self) {
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            ps.editing = None;
        }
    }

    /// Commit the edit buffer to the selected field, then refresh it.
    pub(crate) fn plugin_settings_commit_edit(&mut self) {
        let Some(Overlay::PluginSettings(ref ps)) = self.overlay else {
            return;
        };
        let Some(buf) = ps.editing.clone() else {
            return;
        };
        let Some(&(ei, fi)) = ps.rows.get(ps.selected) else {
            return;
        };
        self.plugin_settings_write(ei, fi, &buf);
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            ps.editing = None;
        }
    }

    /// Write `raw` to field `(ei, fi)` — config coerced to its type into the
    /// lockfile, secret into the keychain — and re-describe that plugin so the
    /// overlay reflects the new state. Errors land in the overlay message line.
    fn plugin_settings_write(&mut self, ei: usize, fi: usize, raw: &str) {
        let Some(Overlay::PluginSettings(ref ps)) = self.overlay else {
            return;
        };
        let Some(entry) = ps.entries.get(ei) else {
            return;
        };
        let Some(field) = entry.fields.get(fi) else {
            return;
        };
        let plugin_id = entry.plugin_id.clone();
        let key = field.key.clone();
        let store = KeyringStore::new();

        let result = if field.secret {
            if raw.is_empty() {
                Err("empty value — nothing stored".to_string())
            } else {
                settings::set_secret(&plugin_id, &key, raw, &store).map_err(|e| e.to_string())
            }
        } else {
            settings::coerce(field.kind, raw)
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    settings::set_config(&self.workspace_root, &plugin_id, &key, v)
                        .map_err(|e| e.to_string())
                })
        };

        // Re-describe the plugin so `value` / `isSet` reflect the write.
        let refreshed = settings::describe(&self.workspace_root, &plugin_id, &store).ok();
        if let Some(Overlay::PluginSettings(ref mut ps)) = self.overlay {
            if let (Some(fields), Some(entry)) = (refreshed, ps.entries.get_mut(ei)) {
                entry.fields = fields;
            }
            ps.message = Some(match result {
                Ok(()) => format!("saved {plugin_id}/{key}"),
                Err(e) => e,
            });
        }
    }
}

/// Render a config value as a plain string — the text a user sees and edits
/// (strings unquoted, everything else as JSON). Shared with the overlay view.
pub(crate) fn value_to_input(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// `outl-plugins` is a hard dependency with its default `js` feature on,
// so the Boa engine is always present in a TUI build — no feature gate.
#[cfg(test)]
mod tests {
    use super::{collect_transform_jobs, fence_lang_and_body, resolve_transformer};
    use crate::state::App;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::workspace::Workspace;
    use outl_md::parse::OutlineNode;
    use std::collections::HashMap;
    use std::path::Path;
    use tempfile::TempDir;

    fn node(text: &str) -> OutlineNode {
        OutlineNode {
            text: text.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn fence_lang_and_body_extracts_raw_lang_and_body() {
        // Raw lang is returned verbatim; body keeps interior newlines.
        let (lang, body) = fence_lang_and_body("```rs\nlet x = 1;\nlet y = 2;\n```").unwrap();
        assert_eq!(lang, "rs");
        assert_eq!(body, "let x = 1;\nlet y = 2;");
    }

    #[test]
    fn fence_lang_and_body_keeps_custom_lang() {
        // A custom language (no canonical alias) is preserved verbatim
        // so a plugin transformer registered under it can match.
        let (lang, body) = fence_lang_and_body("```mermaid\ngraph TD;\nA-->B;\n```").unwrap();
        assert_eq!(lang, "mermaid");
        assert_eq!(body, "graph TD;\nA-->B;");
    }

    #[test]
    fn fence_lang_and_body_tolerates_surrounding_whitespace() {
        let (lang, body) = fence_lang_and_body("  ```python\nprint(1)\n```  ").unwrap();
        assert_eq!(lang, "python");
        assert_eq!(body, "print(1)");
    }

    #[test]
    fn fence_lang_and_body_rejects_bare_fence_without_lang() {
        assert!(fence_lang_and_body("```\nplain\n```").is_none());
    }

    #[test]
    fn fence_lang_and_body_rejects_unclosed_fence() {
        assert!(fence_lang_and_body("```rust\nlet x = 1;").is_none());
    }

    #[test]
    fn fence_lang_and_body_rejects_non_fence_text() {
        assert!(fence_lang_and_body("just some prose").is_none());
    }

    #[test]
    fn resolve_transformer_prefers_raw_then_canonical() {
        let mut langs = HashMap::new();
        langs.insert("mermaid".to_string(), "p.diagram".to_string());
        langs.insert("rust".to_string(), "p.fmt".to_string());

        // Custom lang: exact raw match, lang passed through verbatim.
        assert_eq!(
            resolve_transformer("mermaid", &langs),
            Some(("mermaid".to_string(), "p.diagram".to_string()))
        );
        // Alias: no raw `rs` entry, falls back to canonical `rust`.
        assert_eq!(
            resolve_transformer("rs", &langs),
            Some(("rust".to_string(), "p.fmt".to_string()))
        );
        // No transformer for this lang at all.
        assert_eq!(resolve_transformer("python", &langs), None);
    }

    #[test]
    fn collect_jobs_keys_by_flat_dfs_id_and_filters_by_lang() {
        // Tree:  [0] prose
        //        [1] ```mermaid fence (child of 0)
        //        [2] ```lua fence     (root)
        // Only `mermaid` is a registered transformer lang.
        let mut root0 = node("prose");
        root0.children.push(node("```mermaid\ngraph TD;\n```"));
        let root2 = node("```lua\nbar()\n```");
        let blocks = vec![root0, root2];

        let ids = vec![NodeId::new(), NodeId::new(), NodeId::new()];
        let mut langs = HashMap::new();
        langs.insert("mermaid".to_string(), "run.avelino.diagram".to_string());

        let jobs = collect_transform_jobs(&blocks, &ids, &langs);
        assert_eq!(jobs.len(), 1, "only the mermaid fence matches");
        let (id, plugin_id, lang, body) = &jobs[0];
        assert_eq!(*id, ids[1], "DFS preorder: mermaid fence is flat index 1");
        assert_eq!(plugin_id, "run.avelino.diagram");
        assert_eq!(lang, "mermaid");
        assert_eq!(body, "graph TD;");
    }

    #[test]
    fn collect_jobs_empty_when_no_langs() {
        let blocks = vec![node("```rust\nx\n```")];
        let ids = vec![NodeId::new()];
        let jobs = collect_transform_jobs(&blocks, &ids, &HashMap::new());
        assert!(jobs.is_empty());
    }

    /// A dev-mode plugin (no lockfile, permissions implicitly granted)
    /// that contributes a slash command and an `onOp` hook.
    const BUNDLE: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.commands.register('say-hi', () => {
                    ctx.ui.notify('hi from plugin');
                });
                ctx.ops.onOp((op) => {
                    if (op.kind === 'Edit') {
                        ctx.log.info('saw edit on ' + op.node);
                    }
                });
            }
        });
    "#;

    fn write_dev_plugin(root: &Path) {
        let dir = root.join(".outl/plugins/_dev/hello");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            br#"{
                "id": "run.avelino.hello",
                "name": "Hello",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["slash-command", "op-hook"],
                "permissions": ["read-page", "write-page", "submit-op", "read-op-log"],
                "contributes": { "commands": [{ "id": "say-hi", "title": "Say hi" }] }
            }"#,
        )
        .unwrap();
        std::fs::write(dir.join("index.js"), BUNDLE).unwrap();
    }

    fn app_with(root: &TempDir) -> App {
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        App::new(
            root.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap()
    }

    #[test]
    fn no_plugins_dir_still_builds_an_empty_host() {
        let dir = TempDir::new().unwrap();
        let app = app_with(&dir);
        // Best-effort load with no `.outl/plugins`: the host exists but
        // contributes nothing.
        let host = app.plugin_host.as_ref().expect("host built");
        assert!(host.commands().is_empty());
    }

    #[test]
    fn dev_plugin_command_shows_in_slash_menu() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let mut app = app_with(&dir);
        // `App::new` ran `load_plugins`, but the dev plugin was written
        // before construction so it loaded. Re-loading is also fine.
        app.load_plugins();

        app.open_slash();
        let titles: Vec<String> = match &app.overlay {
            Some(crate::state::Overlay::Slash(s)) => {
                s.candidates.iter().map(|c| c.name.clone()).collect()
            }
            _ => panic!("slash overlay should be open"),
        };
        // The slash entry is keyed by the command **id** (`say-hi`), not
        // the human title — see `slash_candidates` for why (CLI / built-in
        // parity + fuzzy discoverability by id).
        assert!(
            titles.iter().any(|t| t == "say-hi"),
            "plugin command missing from slash menu: {titles:?}"
        );
    }

    #[test]
    fn dev_plugin_command_shows_in_inline_slash_autocomplete() {
        // The Insert-mode inline `/` autocomplete is a *separate* path
        // from the Normal-mode slash overlay above — it has its own
        // candidate list (`candidates_for_slash`) and its own dispatch
        // (`accept_slash_inline` → `find_plugin_command`). This is the
        // surface the user actually hits typing `/stats` inside a block,
        // and it used to list built-ins only. Guard both halves.
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let mut app = app_with(&dir);
        app.load_plugins();

        // Candidate list: typing `/say` must surface the plugin command.
        let cands = app.candidates_for_slash("say");
        assert!(
            cands.iter().any(|c| c == "say-hi"),
            "plugin command missing from inline slash autocomplete: {cands:?}"
        );

        // Dispatch resolution: the id resolves back to its plugin so
        // `accept_slash_inline` can run it.
        assert_eq!(
            app.find_plugin_command("say-hi"),
            Some(("run.avelino.hello".to_string(), "say-hi".to_string())),
        );
        assert_eq!(app.find_plugin_command("not-a-command"), None);
    }

    #[test]
    fn installed_plugin_command_shows_in_slash_menu() {
        // The *installed* path — lockfile + bundleHash + capability
        // intersection — not the relaxed `_dev` loader. This is the exact
        // path `outl plugin install` takes, so a break in `load_installed`
        // (hash check, lockfile read, capability gating) is caught here,
        // not only in the dev-mode test above. Regression guard for the
        // "installed a plugin, its command never showed in `/`" report.
        let src = TempDir::new().unwrap();
        std::fs::write(
            src.path().join("plugin.json"),
            br#"{
                "id": "app.outl.examples.stats",
                "name": "Stats",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["slash-command"],
                "permissions": ["read-page"],
                "contributes": { "commands": [{ "id": "stats", "title": "Workspace statistics" }] }
            }"#,
        )
        .unwrap();
        std::fs::write(
            src.path().join("index.js"),
            br#"globalThis.__outl_register({ activate(ctx) { ctx.commands.register('stats', () => {}); } });"#,
        )
        .unwrap();

        // Install into the workspace's `.outl/plugins/` (writes the lockfile
        // + hashes the bundle), then boot the app against that root.
        let ws_root = TempDir::new().unwrap();
        let plugins = outl_plugins::plugins_dir(ws_root.path());
        std::fs::create_dir_all(&plugins).unwrap();
        outl_plugins::install_from_dir(
            &plugins,
            src.path(),
            "local:./stats",
            vec![outl_plugins::Permission::ReadPage],
            None,
        )
        .unwrap();

        let mut app = app_with(&ws_root);
        // `App::new` already ran `load_plugins`; re-running is idempotent.
        app.load_plugins();

        app.open_slash();
        let titles: Vec<String> = match &app.overlay {
            Some(crate::state::Overlay::Slash(s)) => {
                s.candidates.iter().map(|c| c.name.clone()).collect()
            }
            _ => panic!("slash overlay should be open"),
        };
        assert!(
            titles.iter().any(|t| t == "stats"),
            "installed plugin command missing from slash menu: {titles:?}"
        );

        // Reproduce the exact report: typing `/sta` must keep the plugin
        // command in the filtered list (and, keyed by id, rank it on top —
        // not off-screen below the date built-ins).
        if let Some(crate::state::Overlay::Slash(s)) = &mut app.overlay {
            s.query = "sta".to_string();
        }
        app.refresh_slash();
        let top = match &app.overlay {
            Some(crate::state::Overlay::Slash(s)) => s.candidates.first().map(|c| c.name.clone()),
            _ => panic!("slash overlay should be open"),
        };
        assert_eq!(
            top.as_deref(),
            Some("stats"),
            "typing `/sta` should rank the plugin command on top"
        );
    }

    #[test]
    fn op_hook_fires_after_a_workspace_mutation() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let mut app = app_with(&dir);
        app.load_plugins();

        // Apply an edit directly through outl-actions to append an
        // `Edit` op (mirrors what a keystroke commit does), then run
        // the post-mutation hook point. The hook only logs, so the
        // sweep must not error and must leave the host intact.
        let page = outl_actions::page::open_or_create(
            &mut app.workspace,
            &app.hlc,
            "inbox",
            "Inbox",
            outl_actions::page::PageKind::Page,
        )
        .unwrap();
        let block =
            outl_actions::block::create_under(&mut app.workspace, &app.hlc, page, Some("x"))
                .unwrap();
        outl_actions::block::edit_text(&mut app.workspace, &app.hlc, block, "edited").unwrap();

        // Must not panic / drop the host; the log-only hook applies no
        // intents but still runs.
        app.run_plugin_op_hooks();
        assert!(app.plugin_host.is_some(), "host survives the sweep");
    }

    /// A dev plugin that registers a `upcase` content transformer:
    /// it uppercases the fence body and returns a `text` descriptor.
    const TRANSFORMER_BUNDLE: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.content.register('upcase', (input) => {
                    return { kind: 'text', content: input.toUpperCase() };
                });
            }
        });
    "#;

    fn write_transformer_plugin(root: &Path) {
        let dir = root.join(".outl/plugins/_dev/upcase");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            br#"{
                "id": "run.avelino.upcase",
                "name": "Upcase",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["content-transformer:text"],
                "permissions": ["read-page"],
                "contributes": { "transformers": [{ "lang": "upcase", "kind": "text" }] }
            }"#,
        )
        .unwrap();
        std::fs::write(dir.join("index.js"), TRANSFORMER_BUNDLE).unwrap();
    }

    #[test]
    fn recompute_transforms_caches_text_transformer_output() {
        let dir = TempDir::new().unwrap();
        write_transformer_plugin(dir.path());
        let mut app = app_with(&dir);
        app.load_plugins();

        // Stage a parsed page with one `upcase` fence block and a flat
        // id map aligned to it (what `load_current_no_autorun` builds).
        let id = NodeId::new();
        app.page.blocks = vec![node("```upcase\nhello world\n```")];
        app.id_by_flat = vec![id];

        app.recompute_transforms();

        assert_eq!(
            app.transform_cache.get(&id).map(String::as_str),
            Some("HELLO WORLD"),
            "fence body should be uppercased by the transformer"
        );
    }

    #[test]
    fn recompute_transforms_ignores_fences_with_no_transformer() {
        let dir = TempDir::new().unwrap();
        write_transformer_plugin(dir.path());
        let mut app = app_with(&dir);
        app.load_plugins();

        // A `rust` fence has no registered transformer — nothing cached.
        let id = NodeId::new();
        app.page.blocks = vec![node("```rust\nlet x = 1;\n```")];
        app.id_by_flat = vec![id];

        app.recompute_transforms();
        assert!(
            app.transform_cache.is_empty(),
            "no transformer for `rust` → cache stays empty"
        );
    }
}
