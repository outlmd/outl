//! Theme commands — thin wrappers over
//! `outl_tauri_shared::commands::theme`. The bodies live in the
//! shared crate so mobile registers the same two commands (RFC 0022).

use outl_tauri_shared::commands::theme::ThemeConfigDto;
use outl_theme::Palette;

#[tauri::command]
pub(crate) fn list_themes() -> Vec<String> {
    outl_tauri_shared::commands::theme::list_themes()
}

#[tauri::command]
pub(crate) fn get_theme(name: Option<String>) -> Palette {
    outl_tauri_shared::commands::theme::get_theme(name)
}

#[tauri::command]
pub(crate) fn get_theme_config() -> ThemeConfigDto {
    outl_tauri_shared::commands::theme::get_theme_config()
}
