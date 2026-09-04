//! Page / journal navigation commands — thin wrappers over
//! `outl_tauri_shared::commands::page`, plus the mobile-only legacy
//! compat shims at the bottom.

use outl_actions::{
    open_today, page_meta as page_meta_action, read_page_outline_with_workspace, ActionError,
    OutlineNode, PageMeta,
};
use tauri::State;

use crate::state::{AppState, PageView};
use outl_tauri_shared::commands::page as shared;
use outl_tauri_shared::helpers::{with_ws, with_ws_mut};
use outl_tauri_shared::state::{BacklinksReply, BlockHit};

#[tauri::command]
pub(crate) fn list_all_pages(state: State<'_, AppState>) -> Result<Vec<PageMeta>, String> {
    shared::list_all_pages(state.inner())
}

#[tauri::command]
pub(crate) fn search_pages(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PageMeta>, String> {
    shared::search_pages(state.inner(), query)
}

/// Same shape as [`search_pages`], but filtered to `type:: person`
/// pages. Powers the `@` mention autocomplete on mobile.
#[tauri::command]
pub(crate) fn search_persons(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PageMeta>, String> {
    shared::search_persons(state.inner(), query)
}

/// Fuzzy-search block text for the `((…))` block-ref autocomplete.
/// Registered for parity with desktop (shared command body); the mobile
/// UI does not wire the block-ref popup yet.
#[tauri::command]
pub(crate) fn search_blocks(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<BlockHit>, String> {
    shared::search_blocks(state.inner(), query)
}

/// Search the GitHub gemoji catalog for shortcodes matching `query`.
/// Powers the `:shortcode:` autocomplete in the mobile block editor.
#[tauri::command]
pub(crate) fn outl_emoji_search(
    query: String,
    limit: usize,
) -> Result<Vec<outl_md::emoji::EmojiHit>, String> {
    shared::emoji_search(query, limit)
}

#[tauri::command]
pub(crate) fn open_today_journal(state: State<'_, AppState>) -> Result<PageView, String> {
    shared::open_today_journal(state.inner())
}

#[tauri::command]
pub(crate) fn open_journal_for(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_journal_for(state.inner(), slug)
}

#[tauri::command]
pub(crate) fn open_page_by_slug(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_page_by_slug(state.inner(), slug)
}

/// Open whatever a user-typed ref / tag / picker entry points at, in
/// one round-trip — see the shared body for the single decision tree.
#[tauri::command]
pub(crate) fn open_ref(
    target: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_ref(state.inner(), &app, target)
}

#[tauri::command]
pub(crate) fn previous_day(slug: String) -> Result<String, String> {
    shared::previous_day(slug)
}

#[tauri::command]
pub(crate) fn next_day(slug: String) -> Result<String, String> {
    shared::next_day(slug)
}

#[tauri::command]
pub(crate) fn today_slug_cmd() -> String {
    shared::today_slug()
}

#[tauri::command]
pub(crate) fn date_title(slug: String) -> Result<String, String> {
    shared::date_title(slug)
}

#[tauri::command]
pub(crate) fn resolve_ref(
    target: String,
    state: State<'_, AppState>,
) -> Result<Option<PageMeta>, String> {
    shared::resolve_ref(state.inner(), target)
}

#[tauri::command]
pub(crate) fn toggle_pin(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    shared::toggle_pin(state.inner(), page_id)
}

/// Delete a page by slug. Caller confirms before invoking — this
/// command does not re-prompt. Returns a fresh `PageView` of today's
/// journal so the frontend navigates away from the (now-gone) page.
#[tauri::command]
pub(crate) fn delete_page(slug: String, state: State<'_, AppState>) -> Result<PageView, String> {
    shared::delete_page(state.inner(), slug)
}

/// Compute a page's backlinks lazily, off the page-open path (the
/// O(blocks) scan that used to block the first journal paint). The
/// frontend calls this after the outline renders.
#[tauri::command]
pub(crate) async fn page_backlinks(
    slug: String,
    state: State<'_, AppState>,
) -> Result<BacklinksReply, String> {
    shared::page_backlinks(state.inner(), slug).await
}

/// Persist the backlinks-list direction and return `slug`'s re-sorted
/// backlinks (issue #142). `order` is `"newest"` | `"oldest"`.
#[tauri::command]
pub(crate) async fn set_backlinks_order(
    order: String,
    slug: String,
    state: State<'_, AppState>,
) -> Result<BacklinksReply, String> {
    shared::set_backlinks_order(state.inner(), order, slug).await
}

/// Resolve a batch of block ids to the distinct page/journal slugs they
/// belong to — the sync-progress feed's "page X synced" labels.
#[tauri::command]
pub(crate) fn resolve_page_labels(
    node_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    shared::resolve_page_labels(state.inner(), node_ids)
}

// ---------------------------------------------------------------------------
// Compat shims (LEGACY — mobile-only, deliberately not promoted to the
// shared crate; delete when the old frontends are gone)
// ---------------------------------------------------------------------------

/// Legacy: returns the outline of today's journal so the old frontend
/// that doesn't know about pages still works.
#[tauri::command]
pub(crate) fn list_outline(state: State<'_, AppState>) -> Result<Vec<OutlineNode>, String> {
    let today_id = with_ws_mut(state.inner(), |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(state.inner(), |ws| {
        let meta = page_meta_action(ws, today_id)
            .ok_or_else(|| ActionError::NotInTree(today_id.to_string()))
            .map_err(|e| e.to_string())?;
        read_page_outline_with_workspace(&state.storage_root, &meta, ws)
            .map(|po| po.nodes)
            .map_err(|e| e.to_string())
    })
}

/// Legacy quick capture used by older frontends.
#[tauri::command]
pub(crate) fn add_block(text: String, state: State<'_, AppState>) -> Result<PageView, String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("empty block".to_string());
    }
    let today_id = with_ws_mut(state.inner(), |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    outl_tauri_shared::commands::block::create_block(
        state.inner(),
        today_id.to_string(),
        None,
        None,
        None,
        Some(trimmed),
    )
    .map(|r| r.view)
}
