//! The native application menu, on macOS only.
//!
//! Tauri installs its own default menu on macOS when an app builds none,
//! and that one has nothing outl-specific in it: the File menu holds
//! "Close Window" and little else. The Settings modal tells the user to
//! use "File → Switch Workspace…", so the menu is built here instead,
//! with everything the default offered plus that one item.
//!
//! Windows and Linux get no menu from Tauri, so they get none from us
//! either: a menu bar there would be new chrome nobody asked for. Those
//! platforms switch workspace from the "Switch…" button in Settings,
//! which runs the same frontend flow. [`install`] is a no-op there, and
//! the menu itself is compiled out; `action_for` and the constants stay
//! so Linux CI still runs their tests.
//!
//! This module owns the *menu*, not the behaviour. A click resolves to
//! a [`MenuAction`] and nothing else happens here: Switch Workspace
//! emits [`SWITCH_WORKSPACE_EVENT`] for `lib/switch-workspace.ts`,
//! About hands off to [`crate::about`], and a Help item opens a URL
//! declared in [`crate::links`].
//!
//! Shape follows the default menu it replaces: app menu (About /
//! Services / Hide / Quit), File, Edit, View, Window, plus Help. The
//! Edit menu is what makes `Cmd+C` / `Cmd+V` / `Cmd+Z` reach the
//! webview, so it cannot be left out.
//!
//! **About is ours, not `PredefinedMenuItem::about`.** That one opens
//! AppKit's standard panel, which shows an icon, a name, a version and
//! credits text with no clickable link in it — see [`crate::about`] for
//! what that cost the user and why this is a window.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, WINDOW_SUBMENU_ID};
#[cfg(target_os = "macos")]
use tauri::{AppHandle, Emitter, Runtime};
#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;

use crate::links;

/// Id of the File → Switch Workspace… item.
pub const SWITCH_WORKSPACE_ID: &str = "switch-workspace";

/// Label of the item. `SettingsModal.tsx` names it in its helper text, so
/// the two have to read the same.
pub const SWITCH_WORKSPACE_LABEL: &str = "Switch Workspace…";

/// Event the frontend listens to (`onMenuSwitchWorkspace`).
pub const SWITCH_WORKSPACE_EVENT: &str = "menu://switch-workspace";

/// Id of the app menu's About item.
pub const ABOUT_ID: &str = "about";

/// The Help submenu's title. Named here rather than inline so the
/// doc and the test can point at one constant.
pub const HELP_MENU_TITLE: &str = "Help";

/// What a click on one of our items does. Predefined items (Quit,
/// Copy, …) are handled natively by AppKit and never reach this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Raise an event for the frontend to act on.
    Emit(&'static str),
    /// Open the About window.
    ShowAbout,
    /// Open a URL in the user's browser.
    OpenUrl(&'static str),
}

/// What the menu item `id` should do, if it is one of ours.
fn action_for(id: &str) -> Option<MenuAction> {
    match id {
        SWITCH_WORKSPACE_ID => Some(MenuAction::Emit(SWITCH_WORKSPACE_EVENT)),
        ABOUT_ID => Some(MenuAction::ShowAbout),
        other => links::by_id(other).map(|link| MenuAction::OpenUrl(link.url)),
    }
}

/// Attach the menu and its click handler to the app builder.
#[cfg(target_os = "macos")]
pub fn install(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.menu(build).on_menu_event(on_event)
}

/// No menu off macOS: Tauri installs none by default there, and a menu bar
/// would be new chrome. Settings has the "Switch…" button instead.
#[cfg(not(target_os = "macos"))]
pub fn install(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
}

/// Build the application menu. Passed to `Builder::menu`.
#[cfg(target_os = "macos")]
fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let name = app.package_info().name.clone();

    let about = MenuItem::with_id(app, ABOUT_ID, format!("About {name}"), true, None::<&str>)?;

    let switch_workspace = MenuItem::with_id(
        app,
        SWITCH_WORKSPACE_ID,
        SWITCH_WORKSPACE_LABEL,
        true,
        None::<&str>,
    )?;

    let app_menu = Submenu::with_items(
        app,
        &name,
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &switch_workspace,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[&PredefinedMenuItem::fullscreen(app, None)?],
    )?;

    // The reserved id lets macOS append its list of open windows.
    let window = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    // Built from `links::HELP_LINKS` rather than written out, so the
    // order, the labels and the URLs are the ones the About window
    // shows. A row added there appears here with no edit.
    let help_items = links::HELP_LINKS
        .iter()
        .map(|link| MenuItem::with_id(app, link.id, link.label, true, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let help_refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = help_items
        .iter()
        .map(|item| item as &dyn tauri::menu::IsMenuItem<R>)
        .collect();
    let help = Submenu::with_items(app, HELP_MENU_TITLE, true, &help_refs)?;

    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window, &help])
}

/// Route a menu click. Passed to `Builder::on_menu_event`.
#[cfg(target_os = "macos")]
fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match action_for(&event.id().0) {
        Some(MenuAction::Emit(name)) => {
            if let Err(err) = app.emit(name, ()) {
                tracing::warn!("menu: failed to emit {name}: {err}");
            }
        }
        Some(MenuAction::ShowAbout) => crate::about::show(app),
        Some(MenuAction::OpenUrl(url)) => {
            // Best-effort, like every other menu click: a browser that
            // refuses to launch is logged, never a dialog over the app.
            if let Err(err) = app.opener().open_url(url, None::<&str>) {
                tracing::warn!("menu: failed to open {url}: {err}");
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_workspace_item_raises_its_event() {
        assert_eq!(
            action_for(SWITCH_WORKSPACE_ID),
            Some(MenuAction::Emit(SWITCH_WORKSPACE_EVENT)),
        );
    }

    #[test]
    fn the_about_item_opens_the_about_window() {
        assert_eq!(action_for(ABOUT_ID), Some(MenuAction::ShowAbout));
    }

    /// Every Help row routes, and routes to its own URL. An id that
    /// resolves to nothing is a menu item that looks enabled and does
    /// nothing when clicked — the dead-key shape invariant 12 exists
    /// to stop.
    #[test]
    fn every_help_item_opens_its_own_link() {
        for link in links::HELP_LINKS {
            assert_eq!(action_for(link.id), Some(MenuAction::OpenUrl(link.url)));
        }
    }

    #[test]
    fn an_id_that_is_not_ours_does_nothing() {
        assert_eq!(action_for("quit"), None);
        assert_eq!(action_for(""), None);
    }

    /// The Settings modal and the frontend listener spell these out by
    /// hand; this pins the Rust side so a rename here cannot pass unseen.
    #[test]
    fn the_label_and_event_are_the_ones_the_frontend_expects() {
        assert_eq!(SWITCH_WORKSPACE_LABEL, "Switch Workspace…");
        assert_eq!(SWITCH_WORKSPACE_EVENT, "menu://switch-workspace");
    }
}
