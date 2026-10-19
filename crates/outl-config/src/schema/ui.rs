//! What a client renders, and how it takes editing input.
//!
//! Pure view state: every preference here changes what the user sees or
//! how the chrome responds, never what the workspace contains. None of
//! it converges between devices (root `CLAUDE.md` invariant #7).

use serde::{Deserialize, Deserializer, Serialize};

/// Direction of the backlinks ("Linked from") list.
///
/// `lowercase` serde so the TOML reads `backlinks_order = "newest"` /
/// `"oldest"` — how a user thinks of it, not the Rust variant casing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BacklinksOrder {
    /// Most recently referenced page first (the product default).
    #[default]
    Newest,
    /// Oldest referenced page first.
    Oldest,
}

impl BacklinksOrder {
    /// Whether the newest reference sorts to the top — the `bool` the
    /// `outl_actions::sort_backlinks` renderer path expects.
    pub fn newest_first(self) -> bool {
        matches!(self, BacklinksOrder::Newest)
    }
}

/// Display section — cross-client presentation preferences that are
/// pure view state (they never converge between devices, same policy
/// as [`ThemeCfg`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayCfg {
    /// Direction of the backlinks list. Read by every renderer (TUI at
    /// boot, the GUI clients when building a `PageView`). Default
    /// [`BacklinksOrder::Newest`] — the fix for issue #142, where long
    /// backlink lists buried the latest reference at the bottom.
    pub backlinks_order: BacklinksOrder,
}

/// Which side of the light/dark preset pair to render.
///
/// Names a *side*, not a colour: nothing stops a user putting a dark
/// preset in `preset`, and then `Light` returns it. That is a
/// misconfigured pair rather than a resolution bug — `outl doctor`
/// reports it (RFC 0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Always `preset`.
    Light,
    /// Always `ThemeCfg::dark()`.
    Dark,
    /// Follow the OS appearance. The TUI cannot read it and treats
    /// this as `Dark` — see `docs/theming.md` → "Light / dark pair
    /// and `mode`".
    #[default]
    Auto,
}

/// Theme section. Preset names match `outl_theme::PRESETS`
/// (`"outl"`, `"dracula"`, …); unknown names fall back to
/// `outl_theme::default()` at render time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeCfg {
    /// The light side of the pair, and the only preset used when
    /// `mode` is `Light`.
    pub preset: String,
    /// The dark side. `None` resolves to [`ThemeCfg::preset`] — that
    /// fallback is what keeps every pre-RFC-0022 config (which has
    /// only `preset`) behaving exactly as it did.
    pub preset_dark: Option<String>,
    pub mode: ThemeMode,
}

#[derive(Deserialize)]
struct ThemeCfgWire {
    preset: Option<String>,
    preset_dark: Option<String>,
    mode: Option<ThemeMode>,
}

impl<'de> Deserialize<'de> for ThemeCfg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ThemeCfgWire::deserialize(deserializer)?;
        let defaults = Self::default();
        let preset_was_explicit = wire.preset.is_some();
        Ok(Self {
            preset: wire.preset.unwrap_or(defaults.preset),
            // A present section with an explicit legacy `preset` but no
            // `preset_dark` must keep one-preset behaviour. A present empty
            // section is equivalent to a fresh config and gets the brand pair.
            preset_dark: if preset_was_explicit {
                wire.preset_dark
            } else {
                wire.preset_dark.or(defaults.preset_dark)
            },
            mode: wire.mode.unwrap_or(defaults.mode),
        })
    }
}

impl ThemeCfg {
    /// The dark-side preset name, falling back to [`Self::preset`]
    /// when the user never configured a pair.
    ///
    /// Do not inline this as `preset_dark.unwrap_or_default()` — an
    /// empty string is not a preset name and would silently resolve
    /// to `outl_theme::default()`, changing the theme of every
    /// existing config.
    pub fn dark(&self) -> &str {
        self.preset_dark.as_deref().unwrap_or(&self.preset)
    }
}

impl Default for ThemeCfg {
    fn default() -> Self {
        Self {
            preset: "outl-light".to_string(),
            preset_dark: Some("outl".to_string()),
            mode: ThemeMode::Auto,
        }
    }
}

/// How a GUI client paints a mermaid fence.
///
/// Device-local, like the rest of `[editor]`. Two devices may
/// disagree and that is fine: the fence on disk does not change.
/// The TUI ignores this and keeps the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiagramsMode {
    /// Render a mermaid fence with Pele.
    #[default]
    Pele,
    /// Highlight the fence as source.
    Off,
}

/// Editor preferences. `vim_mode` defaults to `true` because
/// outl is keyboard-first — the same default the TUI ships with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorCfg {
    /// Vim-style modal bindings (Normal / Insert / Visual).
    /// When `false`, the desktop falls back to plain text-editing
    /// chrome (no modes; OS-standard chords only). The TUI is
    /// vim-style by definition and ignores this flag.
    pub vim_mode: bool,

    /// Base font size for the outline view (pixels). The TUI
    /// doesn't read this; terminal font is the user's terminal
    /// setting.
    pub font_size: u32,

    /// Mermaid fences in the desktop and mobile apps.
    /// `"pele"` (default) draws them. `"off"` leaves the source.
    pub diagrams: DiagramsMode,
}

impl Default for EditorCfg {
    fn default() -> Self {
        Self {
            vim_mode: true,
            font_size: 15,
            diagrams: DiagramsMode::Pele,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn missing_diagrams_stays_pele() {
        let c: Config = toml::from_str("[editor]\nvim_mode = false\n").unwrap();
        assert_eq!(c.editor.diagrams, DiagramsMode::Pele);
        assert!(!c.editor.vim_mode);
    }

    #[test]
    fn unknown_diagrams_value_fails_to_parse() {
        let err = toml::from_str::<Config>("[editor]\ndiagrams = \"yes\"\n").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("diagrams") || message.contains("unknown"),
            "{message}"
        );
    }

    #[test]
    fn diagrams_off_parses() {
        let c: Config = toml::from_str("[editor]\ndiagrams = \"off\"\n").unwrap();
        assert_eq!(c.editor.diagrams, DiagramsMode::Off);
        assert!(c.editor.vim_mode);
        assert_eq!(c.editor.font_size, 15);
    }

    #[test]
    fn display_section_parses_backlinks_order() {
        let c: Config = toml::from_str("[display]\nbacklinks_order = \"oldest\"\n").unwrap();
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Oldest);
        assert!(!c.display.backlinks_order.newest_first());
    }

    #[test]
    fn missing_display_section_defaults_to_newest() {
        // Only [theme] populated → display falls back to its default
        // (newest-first), so an older config keeps the issue-#142 fix.
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
    }

    #[test]
    fn a_config_with_only_preset_behaves_exactly_as_before() {
        // RFC 0022's whole backwards-compatibility story. `mode`
        // defaults to auto, but with no `preset_dark` both sides of
        // the pair are the same preset, so auto alternates between
        // dracula and dracula — today's behaviour, byte for byte.
        // If someone "simplifies" the None fallback away, every
        // pre-RFC config silently starts theme-switching.
        let c: Config = toml::from_str("[theme]\npreset = \"dracula\"\n").unwrap();
        assert_eq!(c.theme.preset, "dracula");
        assert_eq!(c.theme.dark(), "dracula");
        assert_eq!(c.theme.mode, ThemeMode::Auto);
    }

    #[test]
    fn a_pair_keeps_the_two_sides_apart() {
        let c: Config = toml::from_str(
            "[theme]\nmode = \"auto\"\npreset = \"logseq-light\"\npreset_dark = \"outl\"\n",
        )
        .unwrap();
        assert_eq!(c.theme.preset, "logseq-light");
        assert_eq!(c.theme.dark(), "outl");
        assert_eq!(c.theme.mode, ThemeMode::Auto);
    }

    #[test]
    fn mode_round_trips_through_toml() {
        for (text, want) in [
            ("light", ThemeMode::Light),
            ("dark", ThemeMode::Dark),
            ("auto", ThemeMode::Auto),
        ] {
            let c: Config = toml::from_str(&format!("[theme]\nmode = \"{text}\"\n")).unwrap();
            assert_eq!(c.theme.mode, want);
            let back = toml::to_string(&c).unwrap();
            assert!(back.contains(&format!("mode = \"{text}\"")), "{back}");
        }
    }
}
