//! The `(chord, action)` catalog and per-client support matrix.
//!
//! Bodies live in `outl_tauri_shared::commands::shortcuts` so mobile
//! registers the same two commands (root `CLAUDE.md` invariant 12 — the
//! help overlay can only say *where* an action exists if every client
//! can read the matrix).
outl_tauri_shared::shortcut_commands!(crate::state::AppState);
