//! `outl://` deep links: the cold-start buffer, the parse, and the warm
//! dispatch.
//!
//! The scheme contract itself is `outl_actions::parse_deep_link`, shared
//! with mobile so the two clients cannot drift on what `outl://daily/…`
//! means. What lives here is only this client's two delivery paths and
//! the state between them.
//!
//! Lifted out of `lib.rs` when that file crossed the size ratchet; it is
//! the same code, in the module the rest of the crate's surfaces already
//! use as a pattern (`open_with`, `quick_capture`).

use tauri::{Emitter, Manager};

/// A deep link that arrived during cold start, before the frontend
/// mounted its `deep-link://navigate` listener. The frontend drains it
/// once on boot via [`take_pending_deep_link`] (issue #98).
///
/// Only the **cold-start** path (the launch URL) populates this. The
/// warm path (`on_open_url` while the app runs) emits the event directly
/// — the listener is already up — and never touches the buffer, so a
/// stale target can't replay on the next plain launch.
pub struct PendingDeepLink(pub parking_lot::Mutex<Option<serde_json::Value>>);

/// Frontend command: take (and clear) the deep link buffered during cold
/// start. Returns `null` when the app launched normally.
#[tauri::command]
pub fn take_pending_deep_link(
    pending: tauri::State<'_, PendingDeepLink>,
) -> Option<serde_json::Value> {
    pending.0.lock().take()
}

/// Parse an `outl://` URL via the shared `outl_actions` parser into the
/// `{kind, …}` payload the frontend maps onto its `open*` commands.
///
/// A malformed URL (wrong scheme, unknown kind, bad date, traversal
/// slug) is logged at `warn` and returns `None` — never a crash, never a
/// stray page (issue #98). The parser is shared with `outl-mobile` so
/// the two clients can't drift on the scheme contract.
pub fn deep_link_payload(raw: &str) -> Option<serde_json::Value> {
    use outl_actions::DeepLinkTarget;

    match outl_actions::parse_deep_link(raw) {
        Ok(DeepLinkTarget::Today) => Some(serde_json::json!({ "kind": "today" })),
        Ok(DeepLinkTarget::Capture) => Some(serde_json::json!({ "kind": "capture" })),
        Ok(DeepLinkTarget::Daily(date)) => Some(serde_json::json!({
            "kind": "daily",
            "date": date.format("%Y-%m-%d").to_string(),
        })),
        Ok(DeepLinkTarget::Page(slug)) => Some(serde_json::json!({
            "kind": "page",
            "slug": slug,
        })),
        Err(err) => {
            tracing::warn!("deep link ignored ({raw}): {err}");
            None
        }
    }
}

/// Warm path: an `outl://` URL opened while the app is running. Emit the
/// navigate event (the frontend listener is up) and focus the window.
pub fn dispatch_deep_link(app: &tauri::AppHandle, raw: &str) {
    let Some(payload) = deep_link_payload(raw) else {
        return;
    };
    if let Err(err) = app.emit("deep-link://navigate", payload) {
        tracing::warn!("deep link: failed to emit navigate event: {err}");
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
}
