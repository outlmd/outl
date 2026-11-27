//! Reading a deletion back (issue #287).
//!
//! `TrashSheet.tsx` reads these, reached from Settings → Trash and the
//! palette's "Open trash"; `Capability::Trash` records mobile as `Full`,
//! and that catalog, not this file, owns which clients reach the trash.
outl_tauri_shared::trash_commands!(crate::state::AppState);
