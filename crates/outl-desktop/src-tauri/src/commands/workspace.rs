//! Workspace lifecycle commands: pick / current / reload / stats.

use std::path::PathBuf;

use tauri::{Emitter, State};
use tracing::warn;

use crate::fs_watcher;
use crate::helpers::storage_root_or_err;
use crate::settings::{self, Settings};
use crate::state::{AppState, WorkspaceSummary};
use crate::workspace_open::{open_workspace_at, spawn_background_reconcile};
use outl_tauri_shared::workspace_reload::reload_workspace_into;

/// Pick a directory as the active workspace.
///
/// Frontend calls this after the user accepts a path from the
/// `@tauri-apps/plugin-dialog` file picker. The path is validated
/// (directories created if missing), the workspace is opened, and the
/// choice is persisted in `settings.json` so subsequent launches skip
/// the picker.
///
/// Emits `workspace-ready` when the swap is complete so the frontend
/// can render the outline.
#[tauri::command]
pub(crate) fn set_workspace(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let path = PathBuf::from(&path);
    let lru_cap = outl_config::load().storage.lru_cap;
    // `open_workspace_at` owns the lock slot: it releases the guards
    // of the workspace being replaced (including a re-pick of this very
    // root) and installs the new ones only once the open succeeded.
    let workspace = open_workspace_at(
        state.hlc.actor(),
        &state.hlc,
        &path,
        lru_cap,
        &state.workspace_guards,
    )
    .map_err(|e| format!("open workspace at {}: {e}", path.display()))?;

    *state.workspace.lock() = Some(workspace);
    *state.storage_root.lock() = Some(path.clone());
    // Undo snapshots belong to the previous workspace.
    state.history.lock().clear();
    // As does the backlinks index — drop it so the next `page_backlinks`
    // rebuilds against the new workspace.
    outl_tauri_shared::helpers::invalidate_backlink_index(state.inner());

    // (Re)start the FS watcher for the new root. Dropping the
    // previous handle inside `swap_watcher` stops watching the
    // old directory.
    match fs_watcher::start_watcher(&path, state.hlc.actor(), app.clone()) {
        Ok(handle) => fs_watcher::swap_watcher(&state.fs_watcher, Some(handle)),
        Err(e) => warn!("fs watcher failed to start for {}: {e}", path.display()),
    }

    // Persist the choice. Failure is logged but not fatal — the
    // workspace is open in memory and the user can keep working.
    {
        let mut s = state.settings.lock();
        s.last_workspace = Some(path.clone());
        if let Err(e) = settings::save(&state.app_config_dir, &s) {
            warn!("could not persist last_workspace: {e}");
        }
    }

    if let Err(e) = app.emit("workspace-ready", ()) {
        warn!("emit workspace-ready: {e}");
    }

    // Re-bind the iroh transport to the new root (best-effort, gated on
    // `[sync] transport = "iroh"`). Shut down any transport bound to the
    // previous workspace first so its background runtime stops. The
    // `notify` watcher (restarted above) keeps covering detection
    // regardless of whether iroh comes up.
    if let Some(prev) = state.iroh_transport.lock().take() {
        prev.shutdown();
    }
    // Drop the stale concrete pairing handle too — it points at the now
    // shut-down transport. `wire_iroh_transport` republishes a fresh one.
    *state.iroh_pairing.lock() = None;
    crate::iroh_sync::wire_iroh_transport(
        &state.iroh_transport,
        &state.iroh_pairing,
        path.clone(),
        state.hlc.actor(),
        app.clone(),
    );

    // Background reconcile: scan + reconcile so the user can start
    // editing today's journal while legacy `.md` files (vim-authored,
    // peer-pushed without sidecar, fixture imports) materialise into
    // the workspace tree behind the scenes. Same policy the boot
    // opener uses — single source of truth in `spawn_background_reconcile`.
    spawn_background_reconcile(
        state.workspace.clone(),
        path,
        state.hlc.clone(),
        app.clone(),
    );
    Ok(())
}

/// Returns the currently active workspace path, or `null` when the
/// user hasn't picked one yet.
#[tauri::command]
pub(crate) fn current_workspace(state: State<'_, AppState>) -> Option<String> {
    state
        .storage_root
        .lock()
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub(crate) fn workspace_stats(state: State<'_, AppState>) -> WorkspaceSummary {
    let guard = state.workspace.lock();
    let storage_root = state
        .storage_root
        .lock()
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    match guard.as_ref() {
        Some(ws) => WorkspaceSummary {
            blocks: ws.tree().node_count(),
            ops: ws.log().len(),
            actor: ws.actor.to_string(),
            storage_root,
            ready: true,
        },
        None => WorkspaceSummary {
            blocks: 0,
            ops: 0,
            actor: state.hlc.actor().to_string(),
            storage_root,
            ready: false,
        },
    }
}

/// Return the current settings (vim_mode, theme, font_size,
/// last_workspace). Frontend uses this to hydrate the SettingsModal.
#[tauri::command]
pub(crate) fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().clone()
}

/// Replace the entire settings struct and persist atomically. Use
/// this over per-field setters so the frontend can edit a draft and
/// commit in one round-trip.
#[tauri::command]
pub(crate) fn update_settings(
    next: Settings,
    state: State<'_, AppState>,
) -> Result<Settings, String> {
    let mut guard = state.settings.lock();
    *guard = next;
    settings::save(&state.app_config_dir, &guard).map_err(|e| format!("save settings: {e}"))?;
    Ok(guard.clone())
}

/// Reload the workspace from disk after a peer change. Called by the
/// frontend whenever the `peer-ops-changed` Tauri event fires.
///
/// The replay itself, and the rule for when its result may be published,
/// live in `outl_tauri_shared::workspace_reload` — mobile runs the same
/// two. What stays here is the desktop's own tail: the `.md` reconcile
/// pass (`app` is passed in only so that background thread can emit
/// `workspace-reconciled` on completion).
#[tauri::command]
pub(crate) async fn reload_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let root = storage_root_or_err(state.inner())?;
    reload_workspace_into(state.inner()).await?;
    // Same split as `set_workspace` and the boot opener — reconcile
    // legacy / peer-pushed `.md` files in the background so the
    // frontend doesn't wait.
    // Idempotent: pages already materialised become no-ops inside
    // `reconcile_md` via the
    // `last_synced_hash == md_hash && pipeline_version >= CURRENT_PIPELINE_VERSION`
    // short-circuit.
    spawn_background_reconcile(state.workspace.clone(), root, state.hlc.clone(), app);
    Ok(())
}
