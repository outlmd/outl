//! The colour of the window before the webview has drawn anything.
//!
//! There are three frames at the start of a launch, and they used to be
//! three different colours:
//!
//! 1. the **native window**, which the OS fills before the webview
//!    exists. With no `backgroundColor` in `tauri.conf.json` that is
//!    white on macOS and Windows, so a dark-theme user opened outl with
//!    a white rectangle;
//! 2. the **first HTML frame**, which `index.html`'s unlayered rule
//!    paints — and, since the inline palette cache, paints in the user's
//!    own colours;
//! 3. the **themed app**, once `get_theme` has answered over IPC.
//!
//! `tauri.conf.json` fixes the common case of (1) with the brand's dark
//! background. This module fixes the rest: a user who picked a light
//! theme gets a light window, from the config on disk, before a single
//! pixel of HTML.
//!
//! # What it cannot know
//!
//! `mode = "auto"` resolves against the OS appearance setting, and that
//! is a question the webview answers (`prefers-color-scheme`), not this
//! process. So `auto` takes the **dark** side here, which is what the
//! window did before this existed — the inline cache in `index.html` is
//! what covers an auto user on a light OS, one frame later. Guessing the
//! other way would be a new flash for every auto user on a dark OS,
//! which is the majority of them.

use tauri::window::Color;
use tauri::Manager;

/// The window background for `cfg`, or `None` when the preset names
/// nothing this build knows.
///
/// Pure over the config and the preset table, so the mode → side
/// mapping is testable without a window. Alpha is always `255`: a
/// translucent window background shows the desktop through the app
/// while it boots.
pub(crate) fn window_background(cfg: &outl_config::ThemeCfg) -> Option<Color> {
    let preset = match cfg.mode {
        outl_config::ThemeMode::Light => cfg.preset.as_str(),
        // `Dark` and `Auto` both take the dark side; see the module doc
        // for why `Auto` is not a guess this process gets to make.
        outl_config::ThemeMode::Dark | outl_config::ThemeMode::Auto => cfg.dark(),
    };
    let palette = outl_theme::by_name(preset)?;
    let (r, g, b) = outl_theme::palette::parse_hex(&palette.bg)?;
    Some(Color(r, g, b, 255))
}

/// Repaint every open window from the theme on disk.
///
/// Best-effort on purpose: a window that refuses keeps the static
/// `backgroundColor` from `tauri.conf.json`, which is one frame of the
/// wrong background and never a failed launch.
pub(crate) fn repaint_windows(app: &tauri::AppHandle, cfg: &outl_config::ThemeCfg) {
    let Some(color) = window_background(cfg) else {
        return;
    };
    for (label, window) in app.webview_windows() {
        if let Err(e) = window.set_background_color(Some(color)) {
            tracing::debug!("window {label}: could not set background: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_config::{ThemeCfg, ThemeMode};

    fn cfg(mode: ThemeMode, preset: &str, dark: Option<&str>) -> ThemeCfg {
        ThemeCfg {
            preset: preset.to_string(),
            preset_dark: dark.map(str::to_string),
            mode,
        }
    }

    /// The case this exists for: a light-theme user must not get the
    /// brand's dark window.
    #[test]
    fn a_light_mode_config_takes_the_light_preset() {
        let light = window_background(&cfg(ThemeMode::Light, "outl-light", Some("outl")))
            .expect("outl-light is a built-in preset");
        let dark = window_background(&cfg(ThemeMode::Dark, "outl-light", Some("outl")))
            .expect("outl is a built-in preset");
        assert_ne!(light, dark, "both modes resolved to the same colour");
    }

    /// Documented, deliberate, and the one reading someone will try to
    /// "fix" without the module doc: `auto` cannot read the OS
    /// appearance from this process, so it matches `dark`.
    #[test]
    fn auto_matches_dark_because_the_os_setting_is_not_readable_here() {
        let auto = window_background(&cfg(ThemeMode::Auto, "outl-light", Some("outl")));
        let dark = window_background(&cfg(ThemeMode::Dark, "outl-light", Some("outl")));
        assert_eq!(auto, dark);
    }

    /// A pre-RFC-0022 config carries only `preset`; `ThemeCfg::dark`
    /// falls back to it, so the window must not come back `None`.
    #[test]
    fn a_config_with_no_dark_side_still_resolves() {
        assert!(window_background(&cfg(ThemeMode::Auto, "outl-light", None)).is_some());
    }

    /// An unknown preset is the user's typo, and the answer is the
    /// static `backgroundColor` from `tauri.conf.json` rather than a
    /// colour invented here.
    #[test]
    fn an_unknown_preset_leaves_the_window_to_its_static_default() {
        assert!(window_background(&cfg(ThemeMode::Dark, "nope", Some("nope"))).is_none());
    }

    #[test]
    fn the_background_is_always_opaque() {
        let Color(_, _, _, a) = window_background(&cfg(ThemeMode::Dark, "outl", Some("outl")))
            .expect("built-in preset");
        assert_eq!(a, 255, "a translucent window shows the desktop at boot");
    }
}
