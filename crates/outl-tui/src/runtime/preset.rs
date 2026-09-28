//! Which theme preset a launch resolves to.
//!
//! Three sources can name one (`--theme`, the per-workspace config,
//! the global config) and they are tried in that order. The `Auto`
//! mode's verdict is the interesting part: a terminal cannot read the
//! OS appearance setting, so it is pinned to the dark side rather than
//! guessed.

use crate::theme::{self, Theme};

/// Resolve the active theme.
///
/// Precedence (first hit wins):
/// 1. `--theme <preset>` CLI override.
/// 2. `[theme] preset = "..."` in the **per-workspace**
///    `.outl/config.toml`.
/// 3. `[theme] preset = "..."` in the **global**
///    `~/.config/outl/config.toml` (shared with the desktop client
///    via the `outl-config` crate).
/// 4. [`theme::default_theme`].
///
/// An unknown name falls through silently to the next level. The
/// caller can surface the choice via the status line if it cares.
pub(super) fn resolve_theme(
    cli_override: Option<&str>,
    cfg: &toml::Value,
    global: &outl_config::Config,
) -> Theme {
    if let Some(name) = cli_override {
        if let Some(t) = theme::by_name(name) {
            return t;
        }
    }
    if let Some(preset) = cfg
        .get("theme")
        .and_then(|t| t.get("preset"))
        .and_then(|v| v.as_str())
    {
        if let Some(t) = theme::by_name(preset) {
            return t;
        }
    }
    // Global fallback — same TOML file the desktop reads / writes,
    // so changing the theme in the desktop's Settings modal
    // propagates to the next `outl-tui` launch automatically (and
    // vice versa). Passed in pre-loaded so the launch reads the file
    // once for both theme and `[sync]`.
    if let Some(t) = theme::by_name(resolve_preset_name(&global.theme)) {
        return t;
    }
    theme::default_theme()
}

/// Which preset name this config resolves to on the TUI.
///
/// `Auto` means the dark side here: a terminal cannot read the OS
/// appearance setting. Recorded in `docs/theming.md` → "Light / dark
/// pair and `mode`" so the gap is visible rather than surprising
/// (RFC 0022).
fn resolve_preset_name(cfg: &outl_config::ThemeCfg) -> &str {
    match cfg.mode {
        outl_config::ThemeMode::Light => &cfg.preset,
        outl_config::ThemeMode::Dark | outl_config::ThemeMode::Auto => cfg.dark(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_config::{ThemeCfg, ThemeMode};

    #[test]
    fn auto_resolves_to_the_dark_side_on_the_tui() {
        // A terminal cannot read the OS appearance setting, so `auto`
        // picks the dark side. Declared in docs/theming.md → "Light
        // / dark pair and `mode`" — do not "fix" this into a probe
        // without an RFC: OSC 11 is unreliable under tmux and
        // several emulators answer for the wrong pane.
        let cfg = ThemeCfg {
            preset: "logseq-light".into(),
            preset_dark: Some("nord".into()),
            mode: ThemeMode::Auto,
        };
        assert_eq!(resolve_preset_name(&cfg), "nord");
    }

    #[test]
    fn light_and_dark_pick_their_declared_sides() {
        let cfg = ThemeCfg {
            preset: "logseq-light".into(),
            preset_dark: Some("nord".into()),
            mode: ThemeMode::Light,
        };
        assert_eq!(resolve_preset_name(&cfg), "logseq-light");

        let cfg = ThemeCfg {
            mode: ThemeMode::Dark,
            ..cfg
        };
        assert_eq!(resolve_preset_name(&cfg), "nord");
    }

    #[test]
    fn a_legacy_config_still_resolves_to_its_single_preset() {
        let cfg = ThemeCfg {
            preset: "dracula".into(),
            preset_dark: None,
            mode: ThemeMode::Auto,
        };
        assert_eq!(resolve_preset_name(&cfg), "dracula");
    }
}
