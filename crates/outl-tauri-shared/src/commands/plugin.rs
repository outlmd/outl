//! Plugin command bodies that combine the [`PluginService`] with a
//! refreshed page view.
//!
//! The pure-delegation commands (`plugin_list`, `plugin_toolbar`, …) are
//! one-line calls into the shared [`PluginService`] and live directly in
//! the client wrappers; only [`run`] and [`sync_hooks`] have a real body
//! (service call + optional page re-render), so they live here.

use serde::Serialize;

use outl_plugins::settings::{self, SettingsField};
use outl_plugins::KeyringStore;

use crate::helpers::{build_page_view, parse_node_id, with_ws};
use crate::host::AppHost;
use crate::plugin_service::PluginService;
use crate::state::PageView;

/// Reply for `plugin_run`: the plugin's notifications / errors / applied
/// count, plus the refreshed [`PageView`] of the page that was on screen
/// when the command fired (so the frontend can re-render in one trip).
///
/// `view` is `None` when no page id was supplied (e.g. the palette /
/// sheet fired before a page loaded) or the page no longer resolves — a
/// plugin can move blocks off the current page, but the page node itself
/// stays.
#[derive(Debug, Clone, Serialize)]
pub struct PluginRunReply {
    pub applied: usize,
    pub notifications: Vec<String>,
    pub errors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
    /// HTML documents the plugin emitted via `ctx.ui.render` (gated by
    /// the `ui-render` capability). The frontend plays each as an
    /// ephemeral sandboxed iframe overlay — untrusted plugin output.
    pub views: Vec<String>,
}

/// Reply for `plugin_sync_hooks`: the refreshed [`PageView`] when an
/// op-hook mutated the page on screen (`None` on the common no-op path),
/// plus any `ui-render` views the hooks emitted. The views path is the
/// confetti trigger: toggling a block DONE fires `onOp`, the plugin
/// emits HTML, and the frontend plays it as a sandboxed iframe overlay —
/// so it is populated even when no page re-render is needed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginSyncHooksReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
    pub views: Vec<String>,
    /// Failures not attributable to the page currently on screen.
    pub errors: Vec<String>,
}

/// Run a plugin command. `page_id` is the page currently on screen; the
/// reply carries its refreshed [`PageView`] so the frontend re-renders
/// without a second round-trip. A plugin can mutate any page — the whole
/// workspace was re-projected on the plugin thread before this returns —
/// but the view we return is the one the user is looking at.
pub fn run<S: AppHost>(
    state: &S,
    plugins: &PluginService,
    plugin_id: String,
    command_id: String,
    page_id: Option<String>,
) -> Result<PluginRunReply, String> {
    let mut run = plugins.run_command(plugin_id, command_id)?;

    if run.applied > 0 {
        // The plugin mutated the workspace (any page), so the backlinks
        // index is stale — drop it; the next `page_backlinks` rebuilds.
        crate::helpers::invalidate_backlink_index(state);
    }

    // Re-render the on-screen page from the now-mutated + re-projected
    // workspace. Failure to resolve the page is non-fatal: the mutation
    // already landed, the frontend can fall back to its own reload.
    let view = match page_id {
        Some(id) => {
            let page = parse_node_id(&id)?;
            let root = state.storage_root()?;
            let mut view =
                with_ws(state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None);
            for error in run.projection_errors.drain(..) {
                if let Some(current) = view.as_mut().filter(|view| {
                    projection_matches_path(&error, &outl_actions::page_md_path(&root, &view.page))
                }) {
                    apply_projection_error(current, &error);
                } else {
                    run.errors.push(format!(
                        "markdown projection failed after plugin mutation: {}",
                        error.error
                    ));
                }
            }
            view
        }
        None => {
            run.errors
                .extend(run.projection_errors.drain(..).map(|error| {
                    format!(
                        "markdown projection failed after plugin mutation: {}",
                        error.error
                    )
                }));
            None
        }
    };

    Ok(PluginRunReply {
        applied: run.applied,
        notifications: run.notifications,
        errors: run.errors,
        view,
        views: run.views,
    })
}

/// Describe a plugin's settings form: every config/secret field with its type,
/// current value, and (for secrets) whether it is set. Read-only — reads the
/// schema + lockfile + keychain directly, so it needs no plugin thread.
pub fn settings_describe<S: AppHost>(
    state: &S,
    plugin_id: String,
) -> Result<Vec<SettingsField>, String> {
    let root = state.storage_root()?;
    settings::describe(&root, &plugin_id, &KeyringStore::new()).map_err(|e| e.to_string())
}

/// Set a plaintext config field (coerced to its schema type), then reload the
/// host so the running plugin sees the new value. Rejects secret fields —
/// those go through [`secret_set`].
pub fn config_set<S: AppHost>(
    state: &S,
    plugins: &PluginService,
    plugin_id: String,
    key: String,
    value: String,
) -> Result<(), String> {
    let root = state.storage_root()?;
    let fields =
        settings::describe(&root, &plugin_id, &KeyringStore::new()).map_err(|e| e.to_string())?;
    let field = fields
        .iter()
        .find(|f| f.key == key)
        .ok_or_else(|| format!("`{plugin_id}` has no config field `{key}`"))?;
    if field.secret {
        return Err(format!("`{key}` is a secret — use secret_set"));
    }
    let coerced = settings::coerce(field.kind, &value).map_err(|e| e.to_string())?;
    settings::set_config(&root, &plugin_id, &key, coerced).map_err(|e| e.to_string())?;
    plugins.reload()
}

/// Store a secret field's value in the OS keychain. No host reload needed —
/// `ctx.secrets` reads the keychain lazily each turn.
pub fn secret_set(plugin_id: String, key: String, value: String) -> Result<(), String> {
    if value.is_empty() {
        return Err("empty value — nothing stored".to_string());
    }
    settings::set_secret(&plugin_id, &key, &value, &KeyringStore::new()).map_err(|e| e.to_string())
}

/// Delete a secret field's value from the keychain (idempotent).
pub fn secret_remove(plugin_id: String, key: String) -> Result<(), String> {
    settings::delete_secret(&plugin_id, &key, &KeyringStore::new()).map_err(|e| e.to_string())
}

/// Fire the plugins' `onOp` hook sweep after a user mutation.
/// Best-effort op-hooks (root invariant 7's "any state that converges
/// goes through the op log" applies to the *plugin's* writes too — they
/// route through `outl-actions`).
///
/// The frontend calls this once after committing a block mutation. It
/// must be called with **no** workspace lock held by the webview side;
/// the plugin thread locks the workspace to run the hooks. Views ride
/// back even on the no-op-mutation path (a `ui-render` plugin can emit
/// HTML from its `onOp` hook without mutating the workspace); only the
/// page re-render is gated on `applied > 0`.
pub fn sync_hooks<S: AppHost>(
    state: &S,
    plugins: &PluginService,
    page_id: Option<String>,
) -> Result<PluginSyncHooksReply, String> {
    let mut outcome = plugins.sync_hooks();
    let mut errors = Vec::new();
    let view = if outcome.applied > 0 {
        // A plugin mutated the workspace (any page), so the backlinks
        // index is stale — drop it; the next `page_backlinks` rebuilds.
        crate::helpers::invalidate_backlink_index(state);
        match page_id {
            Some(id) => {
                let page = parse_node_id(&id)?;
                let root = state.storage_root()?;
                let mut view =
                    with_ws(state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None);
                for error in outcome.projection_errors.drain(..) {
                    if let Some(current) = view.as_mut().filter(|view| {
                        projection_matches_path(
                            &error,
                            &outl_actions::page_md_path(&root, &view.page),
                        )
                    }) {
                        apply_projection_error(current, &error);
                    } else {
                        errors.push(format!(
                            "markdown projection failed after plugin hook mutation: {}",
                            error.error
                        ));
                    }
                }
                view
            }
            None => {
                errors.extend(outcome.projection_errors.drain(..).map(|error| {
                    format!(
                        "markdown projection failed after plugin hook mutation: {}",
                        error.error
                    )
                }));
                None
            }
        }
    } else {
        None
    };
    Ok(PluginSyncHooksReply {
        view,
        views: outcome.views,
        errors,
    })
}

fn projection_matches_path(
    failure: &crate::state::ProjectionFailure,
    current_path: &std::path::Path,
) -> bool {
    failure
        .md_ahead_of_log
        .as_ref()
        .is_some_and(|ahead| current_path.to_string_lossy() == ahead.path)
        || failure
            .path
            .as_deref()
            .is_some_and(|path| current_path.to_string_lossy() == path)
}

fn apply_projection_error(view: &mut PageView, failure: &crate::state::ProjectionFailure) {
    if let Some(ahead) = failure.md_ahead_of_log.clone() {
        view.md_ahead_of_log = Some(ahead);
    } else {
        view.projection_error = Some(failure.error.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::projection_matches_path;
    use outl_actions::ActionError;
    use std::path::Path;

    #[test]
    fn ahead_of_log_matches_only_its_page_path() {
        let error = ActionError::PageMarkdownAheadOfLog {
            path: "/workspace/pages/frozen.md".into(),
            lines: 1,
            sample: "\"offline\"".into(),
        };
        let failure = crate::state::ProjectionFailure::from_error_at(None, &error);

        assert!(projection_matches_path(
            &failure,
            Path::new("/workspace/pages/frozen.md")
        ));
        assert!(!projection_matches_path(
            &failure,
            Path::new("/workspace/pages/current.md")
        ));
    }
}
