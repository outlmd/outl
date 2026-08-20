//! Property-catalogue command wrapper — a thin delegate to
//! `outl_tauri_shared::commands::property`.
//!
//! The mobile Properties sheet asks "which key?" with tappable chips
//! instead of a text field (typing `oura-date` on a phone keyboard is
//! the interaction this feature exists to avoid), and the chips are
//! this list. The ranking itself lives in `outl_actions::known_keys`.

use tauri::State;

use crate::state::AppState;
use outl_tauri_shared::commands::property::{self as shared, PropertyKey};

#[tauri::command]
pub(crate) fn known_property_keys(state: State<'_, AppState>) -> Result<Vec<PropertyKey>, String> {
    shared::known_property_keys(state.inner())
}
