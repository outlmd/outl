//! Property command wrappers — thin delegates to
//! `outl_tauri_shared::commands::{property, reminders}`.
//!
//! `set_block_property` lives next to the `remind::` wrappers in
//! [`super::reminders`] (it grew out of them); the two commands here
//! are the rest of the "create / delete a property" surface: the key
//! catalogue that feeds autocomplete, and the page-level writer.

use tauri::State;

use crate::state::AppState;
use outl_tauri_shared::commands::property::{self as shared, PropertyKey};
use outl_tauri_shared::commands::reminders as reminders_shared;
use outl_tauri_shared::state::PageView;

/// Property keys used anywhere in the workspace, most-used first.
/// Feeds the key autocomplete when a property editor opens.
#[tauri::command]
pub(crate) fn known_property_keys(state: State<'_, AppState>) -> Result<Vec<PropertyKey>, String> {
    shared::known_property_keys(state.inner())
}

/// Set — or, with an empty `value`, clear — a property on the **page**
/// itself. Structural keys (`page-slug`, `page-kind`) are refused
/// upstream; the error reaches the status line.
#[tauri::command]
pub(crate) fn set_page_property(
    page_id: String,
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    reminders_shared::set_page_property(state.inner(), &page_id, &key, &value)
}
