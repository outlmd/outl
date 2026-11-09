//! Reading a deletion back (issue #287).
//!
//! Registered here with no sheet in front of it yet: an unregistered
//! command costs a feature, an unused one costs a symbol. Which clients
//! a user can actually reach the trash from is `Capability::Trash`, not
//! this file.
outl_tauri_shared::trash_commands!(crate::state::AppState);
