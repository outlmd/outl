//! `remind::` command wrappers — thin delegates to
//! `outl_tauri_shared::commands::reminders`. Desktop registers the same
//! set; behaviour lives upstream so the two can't drift.
//!
//! Mobile's own piece is [`deliver_due_reminders`]: the same shared
//! "what's due" answer, delivered through iOS's
//! `UNUserNotificationCenter` (via `tauri-plugin-notification`).
//!
//! **App-closed delivery is not covered by this path.** Registering
//! future fires ahead of time with `UNCalendarNotificationTrigger` (and
//! re-filling them from a `BGAppRefreshTask`, since the system caps
//! pending requests at 64) is the follow-up; see `docs/reminders.md` →
//! "Background delivery". What ships today fires whenever the app is
//! running, foreground or background.

use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;
use outl_tauri_shared::commands::reminders::{
    self as shared, ReminderDto, ReminderSettingsDto, SnoozePresetDto,
};
use outl_tauri_shared::reminder_runtime;
use outl_tauri_shared::state::PageView;

#[tauri::command]
pub(crate) fn list_reminders(state: State<'_, AppState>) -> Result<Vec<ReminderDto>, String> {
    shared::list_reminders(state.inner())
}

#[tauri::command]
pub(crate) fn reminder_settings() -> ReminderSettingsDto {
    shared::reminder_settings()
}

#[tauri::command]
pub(crate) fn set_reminder_settings(
    enabled: bool,
    quiet_hours: String,
) -> Result<ReminderSettingsDto, String> {
    shared::set_reminder_settings(enabled, &quiet_hours)
}

#[tauri::command]
pub(crate) fn snooze_reminder(
    block_id: String,
    preset: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    shared::snooze_reminder(state.inner(), &block_id, &preset)
}

#[tauri::command]
pub(crate) fn snooze_presets() -> Vec<SnoozePresetDto> {
    shared::snooze_presets()
}

#[tauri::command]
pub(crate) fn clear_reminder_snooze(
    block_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    shared::clear_reminder_snooze(state.inner(), &block_id)
}

#[tauri::command]
pub(crate) fn set_block_property(
    page_id: String,
    block_id: String,
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::set_block_property(state.inner(), &page_id, &block_id, &key, &value)
}

/// Set (or clear, with an empty value) a property on the **page**
/// itself (`icon::`, `type::`, …). Refuses the structural keys
/// upstream — renaming a page is `page_rename`, not a property edit.
#[tauri::command]
pub(crate) fn set_page_property(
    page_id: String,
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::set_page_property(state.inner(), &page_id, &key, &value)
}

#[tauri::command]
pub(crate) fn set_block_remind(
    page_id: String,
    block_id: String,
    rule: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::set_block_remind(state.inner(), &page_id, &block_id, &rule)
}

#[tauri::command]
pub(crate) fn mark_block_done(
    page_id: String,
    block_id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::mark_block_done(state.inner(), &page_id, &block_id)
}

/// Deliver every reminder that came due, as an OS notification.
#[tauri::command]
pub(crate) fn deliver_due_reminders(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ReminderDto>, String> {
    let due = reminder_runtime::take_due(state.inner());
    for r in &due {
        // One failed banner must not abort the rest, and must not roll
        // back the fired log — that would turn a denied permission into
        // a retry storm the moment it's granted.
        if let Err(e) = app
            .notification()
            .builder()
            .title(format!("outl · {}", r.page_title))
            .body(&r.plain_text)
            .show()
        {
            tracing::warn!("could not show a reminder notification: {e}");
        }
    }
    Ok(due)
}
