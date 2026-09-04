//! The dedicated plugin thread's event loop + marketplace bridges.
//!
//! Split out of [`crate::plugin_service`] so that module stays focused
//! on the request channel / public handle. Everything here runs on the
//! `outl-plugin-host` thread that owns the (`!Send`) Boa `Context`.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_plugins::{load_installed, plugins_dir, ClientCapabilities, MarketplaceItem, PluginHost};
use parking_lot::Mutex;
use tracing::warn;

use crate::host::StorageRootProvider;
use crate::plugin_dto::PluginRunDto;
use crate::plugin_service::{PluginRequest, SyncHooksOutcome};

/// The plugin thread's event loop. Owns the `PluginHost` (and thus the
/// Boa `Context`) for its entire lifetime — nothing here ever moves the
/// host across a thread boundary.
pub(crate) fn run_plugin_thread<R: StorageRootProvider>(
    rx: Receiver<PluginRequest>,
    client: &'static str,
    capabilities: fn() -> ClientCapabilities,
    workspace: Arc<Mutex<Option<Workspace>>>,
    storage_root: R,
    hlc: HlcGenerator,
) {
    let mut host = PluginHost::new(capabilities());
    // The root the host loaded plugins against. `None` until the first
    // successful load; re-loads when the provider reports a different
    // root (a desktop workspace swap — mobile's fixed root never does).
    let mut loaded_root: Option<PathBuf> = None;

    while let Ok(req) = rx.recv() {
        // Marketplace requests skip the lazy load (they touch the
        // lockfile / network, not the host); everything else ensures the
        // host is loaded against the current root first.
        match &req {
            PluginRequest::RegistryList { .. }
            | PluginRequest::InstallOfficial { .. }
            | PluginRequest::SetEnabled { .. }
            | PluginRequest::Uninstall { .. } => {}
            _ => ensure_loaded(
                &mut host,
                capabilities,
                &workspace,
                &storage_root,
                &mut loaded_root,
            ),
        }
        match req {
            PluginRequest::ListCommands { reply } => {
                let cmds = host.commands().into_iter().map(Into::into).collect();
                let _ = reply.send(cmds);
            }
            PluginRequest::ListKeybindings { reply } => {
                let binds = host
                    .keybindings(client)
                    .into_iter()
                    .map(Into::into)
                    .collect();
                let _ = reply.send(binds);
            }
            PluginRequest::ListToolbar { reply } => {
                let buttons = host
                    .toolbar_buttons(client)
                    .into_iter()
                    .map(Into::into)
                    .collect();
                let _ = reply.send(buttons);
            }
            PluginRequest::RunCommand {
                plugin_id,
                command_id,
                reply,
            } => {
                let result = run_command(
                    &mut host,
                    &workspace,
                    &storage_root,
                    &hlc,
                    &plugin_id,
                    &command_id,
                );
                let _ = reply.send(result);
            }
            PluginRequest::SyncHooks { reply } => {
                let outcome = run_sync_hooks(&mut host, &workspace, &storage_root, &hlc);
                let _ = reply.send(outcome);
            }
            PluginRequest::ListTransformers { reply } => {
                let transformers = host.transformers().into_iter().map(Into::into).collect();
                let _ = reply.send(transformers);
            }
            PluginRequest::TransformBlock {
                plugin_id,
                lang,
                input,
                reply,
            } => {
                // Read-only: `transform_block` runs the plugin's JS
                // against the fence body and never mutates the workspace,
                // so no lock and no re-projection.
                let result = host
                    .transform_block(&plugin_id, &lang, &input)
                    .map(|opt| opt.map(Into::into))
                    .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            PluginRequest::RegistryList { reply } => {
                let _ = reply.send(registry_list(&storage_root));
            }
            PluginRequest::InstallOfficial { id, reply } => {
                let result = install_official(&storage_root, &hlc, &id);
                // Force the host to reload so the newly-installed plugin
                // is live on the next request (op-hooks, commands,
                // transformers).
                if result.is_ok() {
                    loaded_root = None;
                }
                let _ = reply.send(result);
            }
            PluginRequest::SetEnabled { id, enabled, reply } => {
                let result = set_enabled(&storage_root, &id, enabled);
                if result.is_ok() {
                    loaded_root = None;
                }
                let _ = reply.send(result);
            }
            PluginRequest::Uninstall { id, reply } => {
                let result = uninstall(&storage_root, &id);
                if result.is_ok() {
                    loaded_root = None;
                }
                let _ = reply.send(result);
            }
            PluginRequest::Reload { reply } => {
                // Drop the loaded host so the next request rebuilds it from the
                // lockfile (picking up a config edit made off-thread).
                loaded_root = None;
                let _ = reply.send(Ok(()));
            }
        }
    }
}

/// Resolve the provider's current root, erroring when no workspace is
/// open. The marketplace functions live in `outl-plugins` (one owner);
/// these thin wrappers only bridge the provider to the shared `&Path`
/// API.
fn root<R: StorageRootProvider>(storage_root: &R) -> Result<PathBuf, String> {
    storage_root
        .current()
        .ok_or_else(|| "no workspace open".to_string())
}

fn registry_list<R: StorageRootProvider>(storage_root: &R) -> Result<Vec<MarketplaceItem>, String> {
    outl_plugins::marketplace_list(&root(storage_root)?).map_err(|e| e.to_string())
}

fn install_official<R: StorageRootProvider>(
    storage_root: &R,
    hlc: &HlcGenerator,
    id: &str,
) -> Result<String, String> {
    outl_plugins::marketplace_install(&root(storage_root)?, &hlc.actor(), id)
        .map_err(|e| e.to_string())
}

fn set_enabled<R: StorageRootProvider>(
    storage_root: &R,
    id: &str,
    enabled: bool,
) -> Result<(), String> {
    outl_plugins::set_enabled(&root(storage_root)?, id, enabled).map_err(|e| e.to_string())
}

fn uninstall<R: StorageRootProvider>(storage_root: &R, id: &str) -> Result<bool, String> {
    outl_plugins::uninstall(&plugins_dir(&root(storage_root)?), id).map_err(|e| e.to_string())
}

/// Load installed plugins the first time both the root and the workspace
/// are available, or re-load when the root has changed (the user swapped
/// folders on the desktop).
///
/// Best-effort: a workspace not yet open (background opener still
/// running) leaves the host empty and tries again on the next request —
/// loading is deliberately gated on the open workspace because
/// `mark_synced` needs the live op log so pre-existing ops don't fire
/// `onOp` hooks at boot. A per-plugin load failure is logged but never
/// blocks the others.
fn ensure_loaded<R: StorageRootProvider>(
    host: &mut PluginHost,
    capabilities: fn() -> ClientCapabilities,
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &R,
    loaded_root: &mut Option<PathBuf>,
) {
    let Some(root) = storage_root.current() else {
        return; // workspace not open yet — retry next request
    };
    if loaded_root.as_ref() == Some(&root) {
        return; // already loaded against this root
    }
    let guard = workspace.lock();
    let Some(ws) = guard.as_ref() else {
        return; // opener still running — retry on the next request
    };

    // A (re)load means a brand-new host: after a swap the old one carries
    // the previous workspace's plugins + `last_seen` cursor.
    let mut fresh = PluginHost::new(capabilities());
    let report = load_installed(&mut fresh, &plugins_dir(&root));
    for (id, err) in &report.failed {
        warn!("plugin {id} failed to load: {err}");
    }
    if !report.loaded.is_empty() {
        tracing::info!("loaded {} plugin(s)", report.loaded.len());
    }

    // Mark synced against the live log so pre-existing ops don't fire
    // `onOp` hooks at boot — only ops produced *after* this point should.
    fresh.mark_synced(ws);

    *host = fresh;
    *loaded_root = Some(root);
}

/// Run a plugin command under the workspace lock, then re-project `.md`
/// if it mutated anything.
fn run_command<R: StorageRootProvider>(
    host: &mut PluginHost,
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &R,
    hlc: &HlcGenerator,
    plugin_id: &str,
    command_id: &str,
) -> Result<PluginRunDto, String> {
    let mut guard = workspace.lock();
    let Some(ws) = guard.as_mut() else {
        return Err("workspace_loading".to_string());
    };
    let run = host
        .run_command(ws, hlc, plugin_id, command_id)
        .map_err(|e| e.to_string())?;
    let mut dto: PluginRunDto = run.into();
    if dto.applied > 0 {
        dto.projection_errors = reproject(ws, storage_root);
    }
    Ok(dto)
}

/// Dispatch the `onOp` sweep under the workspace lock, re-projecting if a
/// hook mutated the workspace. Returns the intents applied + any
/// `ui-render` views the hooks emitted.
fn run_sync_hooks<R: StorageRootProvider>(
    host: &mut PluginHost,
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &R,
    hlc: &HlcGenerator,
) -> SyncHooksOutcome {
    let mut guard = workspace.lock();
    let Some(ws) = guard.as_mut() else {
        return SyncHooksOutcome::default();
    };
    match host.sync_hooks(ws, hlc) {
        Ok(run) => {
            let projection_errors = if run.applied > 0 {
                reproject(ws, storage_root)
            } else {
                Vec::new()
            };
            SyncHooksOutcome {
                applied: run.applied,
                views: run.views,
                projection_errors,
            }
        }
        Err(e) => {
            warn!("plugin op-hook sweep failed: {e}");
            SyncHooksOutcome::default()
        }
    }
}

/// Re-project every page's `.md` after a plugin mutated the in-memory
/// workspace.
///
/// A plugin's intents land in the op log via `outl-actions` →
/// `Workspace::apply`, but **don't** touch the `.md` projection on their
/// own. A plugin can mutate any page (`archive-done` moves blocks to a
/// *different* page), so we render every page from the workspace
/// (`apply_all_pages_md`) — the same "we don't know which pages moved"
/// path the TUI uses. A failure is returned separately from the successful
/// mutation so callers can surface it without claiming the op-log write failed.
fn reproject<R: StorageRootProvider>(
    ws: &Workspace,
    storage_root: &R,
) -> Vec<crate::state::ProjectionFailure> {
    let Some(root) = storage_root.current() else {
        return vec![crate::state::ProjectionFailure {
            path: None,
            md_ahead_of_log: None,
            error: "markdown projection skipped: no workspace storage root is available".into(),
        }];
    };
    outl_actions::apply_all_pages_md(ws, &root)
        .failures
        .into_iter()
        .map(|failure| {
            crate::state::ProjectionFailure::from_error_at(Some(&failure.path), &failure.error)
        })
        .collect()
}
