//! Page / journal navigation commands.
//!
//! Everything except [`open_ref`] is generated from the shared catalog.
//! `open_ref` needs an `AppHandle` — it emits a `ref-projection-failed`
//! event when the target page could not be re-projected — so it is not
//! boilerplate and stays here.
use tauri::State;

use crate::state::{AppState, PageView};
use outl_tauri_shared::commands::page as shared;

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
