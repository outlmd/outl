//! Shortcuts command — return the shared binding catalog so the
//! Solid frontend can wire a single `keydown` listener that maps
//! the user's chord → `Action` → handler.
//!
//! Filtering by mode is a frontend concern: the desktop is a
//! mode-aware editor (Normal/Insert/Visual/Overlay) and the active
//! mode shifts as the user clicks into a textarea, opens the
//! picker, etc. The backend ships every binding; the frontend
//! `lookup(mode, chord)` picks the right one per keystroke.

use outl_shortcuts::Binding;

/// Return every default binding shipped by `outl-shortcuts`. The
/// frontend caches the result on first call and uses it for the
/// rest of the session — bindings never change at runtime today,
/// so a refresh is only needed when the user edits their config
/// (a future feature).
#[tauri::command]
pub(crate) fn list_shortcut_bindings() -> Vec<Binding> {
    outl_shortcuts::default_bindings()
}

/// One client's verdict on one action, in the shape the frontend
/// consumes: a stable tag plus the text to show the user.
#[derive(serde::Serialize)]
pub(crate) struct SupportDto {
    /// `full` | `native` | `partial` | `missing` | `n/a`.
    kind: &'static str,
    /// What the user is told when this client cannot fully deliver
    /// the action. `None` for `full` and `native`.
    why: Option<&'static str>,
}

impl From<outl_shortcuts::Support> for SupportDto {
    fn from(s: outl_shortcuts::Support) -> Self {
        SupportDto {
            kind: s.kind(),
            why: s.nudge(),
        }
    }
}

/// Per-client support for one action.
#[derive(serde::Serialize)]
pub(crate) struct ActionSupportDto {
    /// The `Action`, serialized as `{ "kind": "OpenPicker" }` so the
    /// frontend can key off the same discriminant its handler map
    /// uses.
    action: outl_shortcuts::Action,
    tui: SupportDto,
    desktop: SupportDto,
    mobile: SupportDto,
}

/// Return what every client does with every action in the catalog.
///
/// The desktop needs its own column to replace the `console.warn` it
/// used to emit for an unhandled chord — a warning in DevTools tells
/// the developer something and the user nothing, which is how a
/// missing feature and a broken one became indistinguishable from
/// the keyboard.
///
/// The other two columns ride along because they cost nothing here
/// and let the help overlay say *where* an action does exist, rather
/// than only that it doesn't exist here.
#[tauri::command]
pub(crate) fn list_action_support() -> Vec<ActionSupportDto> {
    outl_shortcuts::Action::ALL
        .iter()
        .map(|action| {
            let s = outl_shortcuts::support(*action);
            ActionSupportDto {
                action: *action,
                tui: s.tui.into(),
                desktop: s.desktop.into(),
                mobile: s.mobile.into(),
            }
        })
        .collect()
}
