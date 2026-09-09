//! Page / journal navigation commands — thin wrappers over
//! `outl_tauri_shared::commands::page`, plus the mobile-only legacy
//! compat shims at the bottom.

use outl_actions::{
    open_today, page_meta as page_meta_action, read_page_outline_with_workspace, ActionError,
    OutlineNode,
};
use tauri::State;

use crate::state::{AppState, PageView};
use outl_tauri_shared::commands::page as shared;
use outl_tauri_shared::helpers::{with_ws, with_ws_mut};

outl_tauri_shared::page_commands!(crate::state::AppState);

/// Open whatever a `[[ref]]` / `((block-ref))` points at.
#[tauri::command]
pub(crate) fn open_ref(
    target: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_ref(state.inner(), &app, target)
}

// ---------------------------------------------------------------------------
// Compat shims (LEGACY — mobile-only, deliberately not promoted to the
// shared crate; delete when the old frontends are gone)
// ---------------------------------------------------------------------------

/// Legacy: returns the outline of today's journal so the old frontend
/// that doesn't know about pages still works.
#[tauri::command]
pub(crate) fn list_outline(state: State<'_, AppState>) -> Result<Vec<OutlineNode>, String> {
    let today_id = with_ws_mut(state.inner(), |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(state.inner(), |ws| {
        let meta = page_meta_action(ws, today_id)
            .ok_or_else(|| ActionError::NotInTree(today_id.to_string()))
            .map_err(|e| e.to_string())?;
        read_page_outline_with_workspace(&state.storage_root, &meta, ws)
            .map(|po| po.nodes)
            .map_err(|e| e.to_string())
    })
}

/// Legacy quick capture used by older frontends.
#[tauri::command]
pub(crate) fn add_block(text: String, state: State<'_, AppState>) -> Result<PageView, String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("empty block".to_string());
    }
    let today_id = with_ws_mut(state.inner(), |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    outl_tauri_shared::commands::block::create_block(
        state.inner(),
        today_id.to_string(),
        None,
        None,
        None,
        Some(trimmed),
    )
    .map(|r| r.view)
}
