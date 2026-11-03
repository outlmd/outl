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
    fallback_theme()
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

/// Does this terminal accept 24-bit colour?
///
/// Pure over the two environment variables that answer it, so the
/// decision is testable without touching the process environment.
///
/// `default_theme` is an RGB preset, and every preset but two maps
/// each palette field through `Color::Rgb`, so an unconfigured launch
/// emitted 24-bit SGR regardless of what the terminal could render
/// ([#453](https://github.com/outlmd/outl/issues/453)). On a 256-colour
/// tmux without `Tc`, or a plain `xterm`, that is approximated or
/// dropped.
///
/// Unlike the OS appearance setting — which has no reliable terminal
/// answer, which is why `Auto` is pinned to dark above — this one has
/// two widely-set signals: `COLORTERM` in (`truecolor`, `24bit`), and
/// a terminfo direct-colour `TERM` entry (`*-direct`).
///
/// Absence is read as "no". A terminal that supports it and sets
/// neither gets the ANSI preset, which renders correctly; the reverse
/// default renders wrong.
fn supports_truecolor(colorterm: Option<&str>, term: Option<&str>) -> bool {
    if let Some(ct) = colorterm {
        let ct = ct.trim().to_ascii_lowercase();
        if ct == "truecolor" || ct == "24bit" {
            return true;
        }
    }
    // terminfo spells direct colour `*-direct`, `*-direct16`,
    // `*-direct256`; `contains` covers all three.
    term.is_some_and(|t| t.contains("-direct"))
}

/// The preset an unconfigured launch lands on.
///
/// Split from the environment read so the choice is testable. Only
/// reached when nothing named a preset: `--theme`, the per-workspace
/// config and the global config are all explicit user choices, and a
/// user who asked for `dracula` on a 256-colour terminal gets
/// `dracula`. Guessing against an explicit request would be the
/// bigger surprise, and `docs/theming.md` already owns that
/// precedence.
fn fallback_for(truecolor: bool) -> Theme {
    if truecolor {
        theme::default_theme()
    } else {
        // Built on `Color::Reset` / ANSI names, so the user's own
        // terminal palette shows through instead of being
        // approximated. DESIGN.md calls that the point of this
        // preset rather than a gap.
        theme::default_dark()
    }
}

/// [`fallback_for`] against the real environment.
fn fallback_theme() -> Theme {
    fallback_for(supports_truecolor(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_config::{ThemeCfg, ThemeMode};

    #[test]
    fn a_truecolor_terminal_keeps_the_brand_preset() {
        assert_eq!(fallback_for(true).name, theme::default_theme().name);
    }

    #[test]
    fn a_terminal_without_truecolor_falls_back_to_the_ansi_preset() {
        let picked = fallback_for(false);
        assert_eq!(picked.name, theme::default_dark().name);
        assert_ne!(
            picked.name,
            theme::default_theme().name,
            "the fallback must differ from the RGB default, or it is not a fallback"
        );
    }

    #[test]
    fn colorterm_truecolor_is_enough() {
        assert!(supports_truecolor(Some("truecolor"), None));
        assert!(supports_truecolor(Some("24bit"), None));
    }

    #[test]
    fn colorterm_is_matched_case_insensitively() {
        assert!(supports_truecolor(Some("TrueColor"), None));
    }

    #[test]
    fn a_direct_colour_terminfo_entry_is_enough() {
        assert!(supports_truecolor(None, Some("xterm-direct")));
        assert!(supports_truecolor(None, Some("tmux-direct")));
    }

    #[test]
    fn a_256_colour_terminal_does_not_claim_truecolor() {
        assert!(!supports_truecolor(None, Some("xterm-256color")));
        assert!(!supports_truecolor(None, Some("screen-256color")));
    }

    #[test]
    fn a_plain_terminal_does_not_claim_truecolor() {
        assert!(!supports_truecolor(None, Some("xterm")));
        assert!(!supports_truecolor(None, Some("linux")));
        assert!(!supports_truecolor(None, None));
    }

    #[test]
    fn an_empty_colorterm_is_not_a_claim() {
        // Exported-but-empty is common in stripped environments.
        assert!(!supports_truecolor(Some(""), Some("xterm")));
    }

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
