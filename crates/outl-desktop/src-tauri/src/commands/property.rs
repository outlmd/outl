//! The property-key catalogue that feeds autocomplete.
//!
//! `set_page_property` and `set_block_property` are registered by
//! `reminder_commands!` — they grew out of the `remind::` writers and
//! still share their shared-crate module.
outl_tauri_shared::property_commands!(crate::state::AppState);
