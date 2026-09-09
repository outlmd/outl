//! Page history read out of the op log.
//!
//! Named `timeline`, not `history`, because [`super::history`] is the
//! undo / redo stack. Two different pasts: that one is *this session's*
//! mutations, this one is the op log's.
outl_tauri_shared::timeline_commands!(crate::state::AppState);
