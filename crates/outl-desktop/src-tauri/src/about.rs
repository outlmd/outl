//! The About window.
//!
//! macOS answers "About outl" with `orderFrontStandardAboutPanel`
//! unless an app says otherwise, and what that panel can show is fixed:
//! the icon, the name, the version and a block of credits text. No
//! link is clickable in it, because `muda` hands AppKit a plain
//! `NSAttributedString`. So the panel told a user their version number
//! and nothing about where the project lives, how to report what just
//! broke, or who pays for it.
//!
//! This is a real window instead, rendering `AboutWindow.tsx` from the
//! same bundle the app already ships — `index.html?view=about`, which
//! `src/index.tsx` branches on before it mounts the app.
//!
//! **A window, not a modal inside the main one.** The modal was the
//! smaller change and it is wrong here: `<AppShell />` only mounts once
//! a workspace is open, so About would have been dead during the boot
//! replay and dead on the first-run picker — the two moments a user is
//! most likely to want the version number and the issue tracker. A
//! window also lands where macOS users expect one: its own entry in the
//! Window menu, `Cmd+W` to close, no workspace required.
//!
//! The window is created on demand and dropped when closed; a second
//! click focuses the one already open rather than stacking a copy.

#[cfg(target_os = "macos")]
use tauri::TitleBarStyle;
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Window label. `capabilities/about.json` scopes that window's
/// permissions by this exact string, so the two have to agree.
pub const ABOUT_WINDOW_LABEL: &str = "about";

/// What the frontend branches on. `src/index.tsx` reads `view` out of
/// the query string and mounts `<AboutWindow />` instead of `<App />`.
pub const ABOUT_WINDOW_URL: &str = "index.html?view=about";

/// Window title. Shown in the Window menu; the window itself hides the
/// title bar's text on macOS so the content starts at the top.
pub const ABOUT_WINDOW_TITLE: &str = "About outl";

/// Inner size, in logical pixels. Fixed: the content is a mark, three
/// lines and three links, and nothing in it reflows usefully.
const ABOUT_WINDOW_SIZE: (f64, f64) = (380.0, 460.0);

/// Show the About window, creating it the first time.
///
/// Never fails the click: a window that cannot be built is logged and
/// the app carries on, the same contract [`crate::quick_capture`]'s
/// registration has.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    if let Some(existing) = app.get_webview_window(ABOUT_WINDOW_LABEL) {
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return;
    }

    let builder = WebviewWindowBuilder::new(
        app,
        ABOUT_WINDOW_LABEL,
        WebviewUrl::App(ABOUT_WINDOW_URL.into()),
    )
    .title(ABOUT_WINDOW_TITLE)
    .inner_size(ABOUT_WINDOW_SIZE.0, ABOUT_WINDOW_SIZE.1)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .center();

    // The content paints its own header, so the title bar carries the
    // traffic lights and nothing else. `AboutWindow.tsx` leaves the top
    // 28px clear for them.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true);

    if let Err(err) = builder.build() {
        tracing::warn!("about: window not opened: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The capability file is matched to the window by label, and a
    /// mismatch is silent: the window opens and every command it calls
    /// is denied, which reads as an About screen stuck on "Loading…".
    #[test]
    fn the_capability_file_scopes_this_window() {
        let capability = include_str!("../capabilities/about.json");
        assert!(
            capability.contains(&format!("\"{ABOUT_WINDOW_LABEL}\"")),
            "capabilities/about.json does not list the {ABOUT_WINDOW_LABEL:?} window",
        );
    }

    /// The frontend branch and this URL are one fact in two languages.
    /// Spelled differently, the window opens the full app instead —
    /// a second workspace view in a 380px box.
    #[test]
    fn the_frontend_branches_on_this_url() {
        let index = include_str!("../../src/index.tsx");
        let query = ABOUT_WINDOW_URL
            .split_once('?')
            .expect("the about URL carries a query string")
            .1;
        let (key, value) = query.split_once('=').expect("the query is key=value");
        assert!(
            index.contains(&format!("\"{key}\"")) && index.contains(&format!("\"{value}\"")),
            "src/index.tsx does not branch on {query:?}",
        );
    }
}
