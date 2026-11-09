//! Workspace lifecycle commands: reload + stats.

use outl_tauri_shared::workspace_reload::reload_workspace_into;
use tauri::State;

use crate::state::{AppState, WorkspaceSummary};

/// Block / op counts and readiness for the frontend's boot gate.
///
/// **Deliberately synchronous**, for the reasons spelled out on the
/// desktop twin (`outl-desktop/src-tauri/src/commands/workspace.rs`):
/// the body is O(1), and Tauri makes an async command with a
/// `State<'_, _>` input return a `Result`, which would change this
/// command's wire shape. Do not convert one of the pair alone.
#[tauri::command]
pub(crate) fn workspace_stats(state: State<'_, AppState>) -> Result<WorkspaceSummary, String> {
    let guard = state.workspace.lock();
    let storage_root = state.storage_root.to_string_lossy().into_owned();
    let summary = match guard.as_ref() {
        Some(ws) => WorkspaceSummary {
            blocks: ws.tree().node_count(),
            ops: ws.total_ops().map_err(|e| e.to_string())?,
            actor: ws.actor.to_string(),
            storage_root,
            ready: true,
        },
        None => WorkspaceSummary {
            blocks: 0,
            ops: 0,
            actor: String::new(),
            storage_root,
            ready: false,
        },
    };
    Ok(summary)
}

/// Reload the workspace from disk after a peer change (the sync poll and
/// the pull-to-refresh both land here).
///
/// The replay itself, and the rule for when its result may be published,
/// live in `outl_tauri_shared::workspace_reload` — the desktop runs the
/// same two.
///
/// A reload replays the WHOLE op log, which on a freshly-synced workspace
/// is 200k+ ops: CPU-bound, seconds long. A synchronous
/// `#[tauri::command]` executes on the Tauri IPC/main worker, so doing the
/// replay inline holds that thread through the whole rebuild and iOS
/// fires the scene-update watchdog (>10s -> SIGKILL) — the "app freezes
/// forever after pairing" bug. The shared helper is `async` and offloads
/// to a blocking pool thread for exactly that reason.
#[tauri::command]
pub(crate) async fn reload_workspace(state: State<'_, AppState>) -> Result<(), String> {
    reload_workspace_into(state.inner()).await
}
