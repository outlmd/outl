//! Undo / redo commands — thin wrappers over
//! `outl_tauri_shared::commands::history`. The body lives in the shared
//! crate (RFC 0254 phase 1); this crate only wires it to
//! `AppState::history`, the same slot the desktop has always had.

use tauri::State;

use crate::state::{AppState, PageView};

/// Revert the last committed mutation on `page_id`. Errors with
/// `"nothing to undo"` when the stack is empty so the frontend can
/// surface it as a toast.
#[tauri::command]
pub(crate) fn undo_page(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    outl_tauri_shared::commands::history::undo_page(state.inner(), page_id)
}

/// Re-apply the mutation the last `undo_page` reverted.
#[tauri::command]
pub(crate) fn redo_page(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    outl_tauri_shared::commands::history::redo_page(state.inner(), page_id)
}
