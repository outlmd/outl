//! Block mutation commands — thin `#[tauri::command]` wrappers over the
//! shared bodies in `outl_tauri_shared::commands::block`. Wire names and
//! reply shapes are the shared crate's contract; nothing is added here.
//!
//! The full desktop surface is registered, including the block-clipboard
//! commands (`move_block_after` / `copy_block_markdown` /
//! `paste_block_after` / `paste_plain_at`) and `create_block`'s optional
//! `before_id` — the frontend can adopt them without a backend change.

use tauri::State;

use crate::state::{AppState, CreateBlockReply, CutBlockReply, PageView};
use outl_tauri_shared::commands::block as shared;

#[tauri::command]
pub(crate) fn create_block(
    page_id: String,
    after_id: Option<String>,
    before_id: Option<String>,
    parent_id: Option<String>,
    text: Option<String>,
    state: State<'_, AppState>,
) -> Result<CreateBlockReply, String> {
    shared::create_block(state.inner(), page_id, after_id, before_id, parent_id, text)
}

#[tauri::command]
pub(crate) fn edit_block(
    page_id: String,
    id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::edit_block(state.inner(), page_id, id, text)
}

#[tauri::command]
pub(crate) fn split_block(
    page_id: String,
    id: String,
    char_offset: u32,
    state: State<'_, AppState>,
) -> Result<CreateBlockReply, String> {
    shared::split_block(state.inner(), page_id, id, char_offset)
}

#[tauri::command]
pub(crate) fn toggle_todo(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::toggle_todo(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn toggle_quote(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::toggle_quote(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn delete_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::delete_block(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn indent_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::indent_block(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn outdent_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::outdent_block(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn move_block_up(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::move_block_up(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn move_block_down(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::move_block_down(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn move_block_after(
    page_id: String,
    id: String,
    after_id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::move_block_after(state.inner(), page_id, id, after_id)
}

#[tauri::command]
pub(crate) fn copy_block_markdown(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    shared::copy_block_markdown(state.inner(), id)
}

#[tauri::command]
pub(crate) fn paste_block_after(
    page_id: String,
    after_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::paste_block_after(state.inner(), page_id, after_id, text)
}

#[tauri::command]
pub(crate) fn set_block_collapsed(
    page_id: String,
    id: String,
    collapsed: bool,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::set_block_collapsed(state.inner(), page_id, id, collapsed)
}

#[tauri::command]
pub(crate) fn paste_markdown_at(
    page_id: String,
    block_id: String,
    caret: u32,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::paste_markdown_at(state.inner(), page_id, block_id, caret, text)
}

#[tauri::command]
pub(crate) fn paste_plain_at(
    page_id: String,
    block_id: String,
    caret: u32,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::paste_plain_at(state.inner(), page_id, block_id, caret, text)
}

#[tauri::command]
pub(crate) fn copy_markdown(
    block_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    shared::copy_markdown(state.inner(), block_ids)
}

#[tauri::command]
pub(crate) fn cut_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<CutBlockReply, String> {
    shared::cut_block(state.inner(), page_id, id)
}

#[tauri::command]
pub(crate) fn copy_block_ref(id: String, state: State<'_, AppState>) -> Result<String, String> {
    shared::copy_block_ref(state.inner(), id)
}
