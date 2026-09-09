//! The `(chord, action)` catalog and per-client support matrix.
//!
//! New on mobile. Root `CLAUDE.md` invariant 12 says a client must be
//! able to tell the user *where* an action exists — which needs the
//! matrix on the device asking the question, not only on the desktop.
outl_tauri_shared::shortcut_commands!(crate::state::AppState);
