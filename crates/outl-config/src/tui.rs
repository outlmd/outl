//! TUI-only preferences and icon selection.

use serde::{Deserialize, Serialize};

/// TUI-only preferences (the desktop ignores this section).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiCfg {
    /// Chrome icon set. Emoji is portable across ordinary terminal fonts;
    /// Nerd Font glyphs are available as an explicit opt-in.
    pub icons: TuiIconStyle,

    /// Capture the mouse so the app owns selection: drag across blocks
    /// selects a range and copies it as clean markdown on release, the
    /// scroll wheel moves the outline selection, a click selects a block.
    ///
    /// Default `false`, and deliberately opt-in: capturing the mouse
    /// **disables the terminal's own text selection** (selecting a URL,
    /// copying a single word, dragging across panes), which is muscle
    /// memory for many terminal users. Turn it on only if you want
    /// mouse-driven copy inside outl more than the terminal's native
    /// selection. The keyboard yank (`yy` / `Y` / Visual `y`) copies
    /// markdown to the clipboard regardless of this flag.
    pub mouse_capture: bool,
}

/// Icon set used by TUI chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TuiIconStyle {
    /// Unicode emoji and symbols supported by ordinary terminal fonts.
    #[default]
    Emoji,
    /// Font Awesome / Material Design glyphs from a Nerd Font.
    NerdFont,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_icon_style_parses_and_defaults_to_emoji() {
        let c: crate::Config = toml::from_str("[tui]\nicons = \"nerd-font\"\n").unwrap();
        assert_eq!(c.tui.icons, TuiIconStyle::NerdFont);

        let c: crate::Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert_eq!(c.tui.icons, TuiIconStyle::Emoji);
    }
}
