//! Page-history command wrapper — a thin delegate to
//! `outl_tauri_shared::commands::timeline`.
//!
//! Named `timeline`, not `history`, because [`super::history`] is the
//! undo / redo stack. Two different pasts: that one is *this session's*
//! mutations, this one is the op log's.

use tauri::State;

use crate::state::AppState;
use outl_tauri_shared::commands::timeline::{self as shared, PageTimelineDto};

/// Every change to a page, newest first, read from the op log.
///
/// Read-only — the panel it feeds shows history, it does not restore.
#[tauri::command]
pub(crate) fn page_timeline(
    page_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<PageTimelineDto, String> {
    shared::page_timeline(state.inner(), page_id, limit)
}
