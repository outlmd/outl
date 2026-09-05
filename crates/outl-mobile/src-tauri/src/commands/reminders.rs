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

/// The notification category every reminder banner is stamped with.
/// See [`reminder_action_catalog`] for why these three live here.
pub(crate) const REMINDER_CATEGORY: &str = "outl.reminder";
/// Snooze the block one hour, converging through `Op::SnoozeRemind`.
pub(crate) const ACTION_SNOOZE_1H: &str = "snooze-1h";
/// Flip the block's `TODO` to `DONE`, which stops every future fire.
pub(crate) const ACTION_DONE: &str = "done";

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
        //
        // The category and the two extras are what make the banner
        // actionable: the category hangs "Snooze 1h" / "Done" off it,
        // and the extras are how the frontend knows *which* block the
        // tap was about. Without them the buttons would arrive with no
        // subject and the handler would have to guess.
        let builder = app
            .notification()
            .builder()
            .title(format!("outl · {}", r.page_title))
            .body(&r.plain_text)
            .extra("blockId", &r.block_id)
            .extra("pageSlug", &r.page_slug);

        #[cfg(mobile)]
        let builder = builder.action_type_id(REMINDER_CATEGORY);

        if let Err(e) = builder.show() {
            tracing::warn!("could not show a reminder notification: {e}");
        }
    }
    Ok(due)
}

/// The category + buttons the frontend registers with the OS on boot.
///
/// **Why the frontend registers something Rust owns.**
/// `tauri-plugin-notification`'s `ActionType` / `Action` are
/// `#[cfg(mobile)]` structs with private fields, no constructor and no
/// `Deserialize`, so a consumer crate cannot build one and
/// `Notification::register_action_types` is unreachable from here even
/// though it exists. The plugin's `registerActionTypes()` in JS reaches
/// the same command over IPC, so that is the only door.
///
/// That leaves the ids needing an owner. Two places must agree on
/// them: [`deliver_due_reminders`] stamps the category onto every
/// banner, and the frontend matches the button that was pressed
/// (`src/lib/reminder-actions.ts`). Handing the frontend the same
/// constants makes a rename one edit instead of a button that silently
/// stops working.
///
/// Mobile-only, and not for want of building it: those `#[cfg(mobile)]`
/// items have no desktop counterpart, and the desktop plugin's `show()`
/// spawns `notify-rust` and drops the handle, so a banner there has
/// nothing to attach a button to. The per-client verdict is declared in
/// `outl_shortcuts::capability_support`
/// (`Capability::ReminderNotificationActions`).
#[derive(serde::Serialize)]
pub(crate) struct ReminderActionCatalog {
    /// Value passed to `action_type_id` on every reminder banner.
    category: &'static str,
    actions: Vec<ReminderActionDto>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReminderActionDto {
    id: &'static str,
    title: &'static str,
    /// What pressing it does, so the frontend dispatches on a declared
    /// kind rather than pattern-matching the id. An id is a wire
    /// value: it can be renamed for the OS without meaning to change
    /// behaviour, and a handler keyed on its spelling would silently
    /// start treating "Snooze" as a tap.
    kind: ReminderActionKind,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReminderActionKind {
    /// Push the next fire out by an hour (`Op::SnoozeRemind`).
    Snooze,
    /// Flip the block to `DONE`, ending the rule.
    Done,
}

/// Hand the frontend the category and buttons to register.
///
/// Registration happens at boot, not on first delivery: iOS resolves a
/// banner's `action_type_id` against the categories known to
/// `UNUserNotificationCenter` **at delivery time**, and a banner naming
/// an unregistered category still shows, just with no buttons and no
/// error. Registering late means the first reminder of every session is
/// the one that silently loses its buttons.
#[tauri::command]
pub(crate) fn reminder_action_catalog() -> ReminderActionCatalog {
    ReminderActionCatalog {
        category: REMINDER_CATEGORY,
        actions: vec![
            ReminderActionDto {
                id: ACTION_SNOOZE_1H,
                title: "Snooze 1h",
                kind: ReminderActionKind::Snooze,
            },
            ReminderActionDto {
                id: ACTION_DONE,
                title: "Done",
                kind: ReminderActionKind::Done,
            },
        ],
    }
}
