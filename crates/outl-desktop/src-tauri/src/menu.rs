//! The native application menu.
//!
//! Tauri installs its own default menu when an app builds none, and that
//! one has nothing outl-specific in it: the File menu holds "Close Window"
//! and little else. The Settings modal tells the user to use
//! "File → Switch Workspace…", so the menu is built here instead, with
//! everything the default offered plus that one item.
//!
//! This module owns the *menu*, not the behaviour. Clicking the item only
//! emits [`SWITCH_WORKSPACE_EVENT`]; the frontend opens the folder dialog
//! and calls `set_workspace`, exactly as the first-run picker does, so no
//! workspace logic lives on this side.
//!
//! Platform shape follows the default menu it replaces:
//!
//! - macOS: app menu (About / Services / Hide / Quit), File, Edit, View,
//!   Window. The Edit menu is what makes `Cmd+C` / `Cmd+V` / `Cmd+Z` reach
//!   the webview, so it cannot be left out.
//! - Windows / Linux: File (with Quit), Edit, Window, Help (About).
//!   Undo and redo are macOS-only here: the platform menu toolkit has no
//!   native undo/redo item elsewhere.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, WINDOW_SUBMENU_ID};
use tauri::{AppHandle, Emitter, Runtime};

/// Id of the File → Switch Workspace… item.
pub const SWITCH_WORKSPACE_ID: &str = "switch-workspace";

/// Label of the item. `SettingsModal.tsx` names it in its helper text, so
/// the two have to read the same.
pub const SWITCH_WORKSPACE_LABEL: &str = "Switch Workspace…";

/// Event the frontend listens to (`onMenuSwitchWorkspace`).
pub const SWITCH_WORKSPACE_EVENT: &str = "menu://switch-workspace";

/// The event a click on the menu item `id` should raise, if it is one of
/// ours. Predefined items (Quit, Copy, …) are handled natively and never
/// reach this.
fn event_for(id: &str) -> Option<&'static str> {
    match id {
        SWITCH_WORKSPACE_ID => Some(SWITCH_WORKSPACE_EVENT),
        _ => None,
    }
}

/// Build the application menu. Passed to `Builder::menu`.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let switch_workspace = MenuItem::with_id(
        app,
        SWITCH_WORKSPACE_ID,
        SWITCH_WORKSPACE_LABEL,
        true,
        None::<&str>,
    )?;

    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &switch_workspace,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::undo(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::redo(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
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
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let menu = {
        let app_menu = Submenu::with_items(
            app,
            &app.package_info().name,
            true,
            &[
                &PredefinedMenuItem::about(app, None, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?;
        let view = Submenu::with_items(
            app,
            "View",
            true,
            &[&PredefinedMenuItem::fullscreen(app, None)?],
        )?;
        Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window])?
    };

    #[cfg(not(target_os = "macos"))]
    let menu = {
        let help = Submenu::with_id_and_items(
            app,
            tauri::menu::HELP_SUBMENU_ID,
            "Help",
            true,
            &[&PredefinedMenuItem::about(app, None, None)?],
        )?;
        Menu::with_items(app, &[&file, &edit, &window, &help])?
    };

    Ok(menu)
}

/// Route a menu click. Passed to `Builder::on_menu_event`.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let Some(name) = event_for(&event.id().0) else {
        return;
    };
    if let Err(err) = app.emit(name, ()) {
        tracing::warn!("menu: failed to emit {name}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_workspace_item_raises_its_event() {
        assert_eq!(event_for(SWITCH_WORKSPACE_ID), Some(SWITCH_WORKSPACE_EVENT));
    }

    #[test]
    fn an_id_that_is_not_ours_raises_nothing() {
        assert_eq!(event_for("quit"), None);
        assert_eq!(event_for(""), None);
    }

    /// The Settings modal and the frontend listener spell these out by
    /// hand; this pins the Rust side so a rename here cannot pass unseen.
    #[test]
    fn the_label_and_event_are_the_ones_the_frontend_expects() {
        assert_eq!(SWITCH_WORKSPACE_LABEL, "Switch Workspace…");
        assert_eq!(SWITCH_WORKSPACE_EVENT, "menu://switch-workspace");
    }
}
