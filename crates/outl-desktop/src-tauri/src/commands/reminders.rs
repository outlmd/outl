//! `remind::` commands.
//!
//! Everything except [`deliver_due_reminders`] is generated from the
//! shared catalog. That one turns the shared "what's due" answer into an
//! actual OS banner through `tauri-plugin-notification`, which needs the
//! client's `AppHandle`.

use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;
use outl_tauri_shared::commands::reminders::ReminderDto;
use outl_tauri_shared::reminder_runtime;

outl_tauri_shared::reminder_commands!(crate::state::AppState);

/// Deliver every reminder that came due, as an OS notification.
///
/// Returns what it delivered so the frontend can also refresh an open
/// Reminders panel without a second round trip. Called on a timer by
/// the frontend rather than from a Rust thread: the webview is the one
/// that knows whether the user is looking at the app, and a timer there
/// pauses with the window instead of buzzing a machine nobody is at.
#[tauri::command]
pub(crate) fn deliver_due_reminders(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ReminderDto>, String> {
    let due = reminder_runtime::take_due(state.inner());
    for r in &due {
        // A failed banner (permission denied, no notification daemon on
        // this Linux session) must not abort the rest — and must not
        // roll back the fired log either, or the user gets a retry storm
        // the moment the daemon comes back.
        if let Err(e) = app
            .notification()
            .builder()
            .title(reminder_title(r))
            .body(&r.plain_text)
            .show()
        {
            tracing::warn!("could not show a reminder notification: {e}");
        }
    }
    Ok(due)
}

/// `outl · <page title>` — the page is the context that makes a bare
/// block body ("call the bank") legible on a lock screen.
fn reminder_title(r: &ReminderDto) -> String {
    format!("outl · {}", r.page_title)
}
