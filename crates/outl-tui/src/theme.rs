//! TUI theming — color palette + ratatui [`Style`]s the renderer applies.
//!
//! A [`Theme`] bundles every styled surface in the TUI under semantic names
//! (`ref_link`, `tag_link`, `bold`, `code`, `cursor_block`, ...) so screen
//! code never references hard-coded colors. Built-in presets cover the
//! common terminal taste palettes; users override via `.outl/config.toml`:
//!
//! ```toml
//! [theme]
//! preset = "dracula"
//! ```
//!
//! Or temporarily via the CLI: `outl --theme nord --path ~/notes`.
//!
//! Adding a preset means adding one constructor function and a string in
//! [`PRESETS`]. The render path doesn't need to change.

use outl_theme::Palette;
use ratatui::style::{Color, Modifier, Style};

/// Convert a `#rrggbb` palette string into a ratatui [`Color`].
///
/// Malformed input degrades to [`Color::Reset`] instead of panicking
/// so a bad config never blocks the TUI from booting. The
/// `outl-theme` crate has a test guard (`every_palette_field_is_hex`)
/// that catches a typo before it gets here.
fn hex_to_color(s: &str) -> Color {
    match outl_theme::palette::parse_hex(s) {
        Some((r, g, b)) => Color::Rgb(r, g, b),
        None => Color::Reset,
    }
}

/// Build a `Theme` from a shared `Palette` plus the static name.
///
/// Modifiers (`BOLD` on `bold`, `UNDERLINED` on links, `ITALIC` on
/// `italic`, `CROSSED_OUT` on `strike`) are consistent across every
/// preset — only the hues vary — so the formula is hard-coded here.
/// A new field in the palette = a new field in `Theme` + a line
/// below.
fn theme_from_palette(name: &'static str, p: &Palette) -> Theme {
    Theme {
        name,
        background: hex_to_color(&p.bg),
        foreground: hex_to_color(&p.fg),
        bullet: Style::default().fg(hex_to_color(&p.fg_dimmer)),
        selected_bullet: Style::default()
            .fg(hex_to_color(&p.selected_bullet_fg))
            .bg(hex_to_color(&p.selected_bullet_bg))
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(hex_to_color(&p.cursor_block_fg))
            .bg(hex_to_color(&p.cursor_block_bg))
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default()
            .fg(hex_to_color(&p.cursor_caret_fg))
            .add_modifier(Modifier::BOLD),
        ref_link: Style::default()
            .fg(hex_to_color(&p.ref_link_fg))
            .add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(hex_to_color(&p.tag_link_fg))
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(hex_to_color(&p.md_link_fg))
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default()
            .fg(hex_to_color(&p.bold_fg))
            .add_modifier(Modifier::BOLD),
        italic: Style::default()
            .fg(hex_to_color(&p.italic_fg))
            .add_modifier(Modifier::ITALIC),
        strike: Style::default()
            .fg(hex_to_color(&p.strike_fg))
            .add_modifier(Modifier::CROSSED_OUT),
        highlight: Style::default()
            .bg(hex_to_color(&p.highlight_bg))
            .fg(hex_to_color(&p.highlight_fg)),
        code: Style::default().fg(hex_to_color(&p.code_fg)),
        todo_open: Style::default()
            .fg(hex_to_color(&p.todo_open_fg))
            .add_modifier(Modifier::BOLD),
        todo_done: Style::default()
            .fg(hex_to_color(&p.todo_done_fg))
            .add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(hex_to_color(&p.todo_done_body_fg))
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(hex_to_color(&p.property_key_fg)),
        property_value: Style::default().fg(hex_to_color(&p.property_value_fg)),
        heading: Style::default()
            .fg(hex_to_color(&p.heading_fg))
            .add_modifier(Modifier::BOLD),
        dim: Style::default().fg(hex_to_color(&p.dim_fg)),
        border: Style::default().fg(hex_to_color(&p.border)),
        hint: Style::default().fg(hex_to_color(&p.hint)),
        status_normal: Style::default()
            .fg(hex_to_color(&p.status_normal_fg))
            .bg(hex_to_color(&p.status_normal_bg))
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(hex_to_color(&p.status_insert_fg))
            .bg(hex_to_color(&p.status_insert_bg))
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(hex_to_color(&p.status_visual_fg))
            .bg(hex_to_color(&p.status_visual_bg))
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(hex_to_color(&p.status_message_fg)),
        help_title: Style::default()
            .fg(hex_to_color(&p.help_title_fg))
            .add_modifier(Modifier::BOLD),
        popup_bg: hex_to_color(&p.bg_elev),
        list_selected: Style::default()
            .fg(hex_to_color(&p.list_selected_fg))
            .bg(hex_to_color(&p.list_selected_bg))
            .add_modifier(Modifier::BOLD),
    }
}

/// All styled surfaces the TUI knows how to paint.
///
/// Add a field here only when there's a *semantic* surface to name. If
/// two surfaces want exactly the same style, share the field. Don't add
/// `inner_bold_in_quote` and other compound names — keep it flat.
#[derive(Debug, Clone)]
pub struct Theme {
    /// User-visible preset name.
    pub name: &'static str,
    /// Background of the alternate screen.
    pub background: Color,
    /// Base body-text color painted across the canvas together with
    /// `background`. `Color::Reset` on the ANSI presets
    /// (`default-dark`, `light`) so the terminal's own colors show
    /// through; a concrete RGB on every palette-derived preset so a
    /// light theme stays readable on a dark terminal (and vice
    /// versa).
    pub foreground: Color,

    // --- bullets / outline ---
    /// Plain `-` bullet when not selected.
    pub bullet: Style,
    /// Bullet of the currently selected block.
    pub selected_bullet: Style,
    /// Single-char block cursor (vim style) in Normal mode.
    pub cursor_block: Style,
    /// Thin caret used at end-of-line and in Insert mode.
    pub cursor_caret: Style,

    // --- inline tokens (consumed by render_markdown_inline / highlight_inline) ---
    /// `[[page]]` references (pretty: name only; raw: with brackets).
    pub ref_link: Style,
    /// `#tag` references.
    pub tag_link: Style,
    /// `[text](url)` standard markdown link (text only in pretty mode).
    pub md_link: Style,
    /// `**bold**` inner text.
    pub bold: Style,
    /// `*italic*` / `_italic_` inner text.
    pub italic: Style,
    /// `~~strike~~` inner text.
    pub strike: Style,
    /// `==highlight==` inner text (Roam `^^highlight^^` on import).
    pub highlight: Style,
    /// `` `code` `` inner text.
    pub code: Style,
    /// Style for unfinished `TODO` prefix on a block.
    pub todo_open: Style,
    /// Style for finished `DONE` prefix on a block.
    pub todo_done: Style,
    /// Body text of a TODO block (carries `dim` for DONE).
    pub todo_done_body: Style,

    // --- structural surfaces ---
    /// `key:: value` property lines (the key part).
    pub property_key: Style,
    /// `key:: value` property lines (the value part).
    pub property_value: Style,
    /// Page heading shown in the main header.
    pub heading: Style,
    /// Dim, used for delimiters in raw render (`**`, `~~`, etc).
    pub dim: Style,

    // --- chrome ---
    /// Borders around panels.
    pub border: Style,
    /// Hint line in the footer.
    pub hint: Style,
    /// Mode badge in Normal mode.
    pub status_normal: Style,
    /// Mode badge in Insert mode.
    pub status_insert: Style,
    /// Mode badge in Visual mode (S2.2).
    pub status_visual: Style,
    /// Transient status messages (saved, error...).
    pub status_message: Style,
    /// Section titles in the help popup.
    pub help_title: Style,
    /// Popup background (quick switcher, command palette, help).
    pub popup_bg: Color,

    // --- accent rails ---
    /// Highlight bar when an item is selected in a list popup.
    pub list_selected: Style,
}

impl Theme {
    /// Base style for elevated surfaces (popups, toasts): `popup_bg`
    /// background plus the theme's base text color. Every overlay
    /// paints this before its content so unstyled spans inherit a
    /// readable foreground instead of the terminal's default — which
    /// may have no contrast against `popup_bg` (light theme on a
    /// dark terminal and vice versa).
    pub fn popup_style(&self) -> Style {
        Style::default().fg(self.foreground).bg(self.popup_bg)
    }
}

/// List of preset names exposed to users (CLI / config / `outl theme list`).
pub const PRESETS: &[&str] = &[
    "outl",
    "outl-light",
    "default-dark",
    "light",
    "logseq-light",
    "dracula",
    "solarized-dark",
    "nord",
    "monokai",
];

/// Look up a preset by name. Case-insensitive; dashes and underscores
/// are interchangeable so `"Solarized Dark"` and `"solarized_dark"`
/// both work.
pub fn by_name(name: &str) -> Option<Theme> {
    let norm: String = name
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            '_' | ' ' => '-',
            _ => c,
        })
        .collect();
    match norm.as_str() {
        "outl" | "default" => Some(outl()),
        "default-dark" | "dark" => Some(default_dark()),
        "light" => Some(light()),
        "logseq-light" | "logseq" => Some(logseq_light()),
        "dracula" => Some(dracula()),
        "solarized-dark" | "solarized" => Some(solarized_dark()),
        "nord" => Some(nord()),
        "monokai" => Some(monokai()),
        "outl-light" => Some(outl_light()),
        _ => None,
    }
}

/// Default fallback when nothing is configured.
pub fn default_theme() -> Theme {
    outl()
}

// --- presets -------------------------------------------------------------

/// outl — the project's brand palette, matched 1:1 with the marketing
/// site (avelino.run). Deep-purple background with a lavender accent
/// and lemon highlight; this is the default theme.
pub fn outl() -> Theme {
    theme_from_palette("outl", &outl_theme::presets::outl())
}

/// outl-light — the brand's light counterpart to [`outl`].
pub fn outl_light() -> Theme {
    theme_from_palette("outl-light", &outl_theme::presets::outl_light())
}

/// Default dark — the original outl-tui palette.
pub fn default_dark() -> Theme {
    Theme {
        name: "default-dark",
        background: Color::Reset,
        foreground: Color::Reset,
        bullet: Style::default().fg(Color::DarkGray),
        selected_bullet: Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        ref_link: Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default().add_modifier(Modifier::BOLD),
        italic: Style::default().add_modifier(Modifier::ITALIC),
        strike: Style::default().add_modifier(Modifier::CROSSED_OUT),
        highlight: Style::default().bg(Color::Yellow).fg(Color::Black),
        code: Style::default().fg(Color::Green),
        todo_open: Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        todo_done: Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(Color::DarkGray),
        property_value: Style::default().fg(Color::Gray),
        heading: Style::default().add_modifier(Modifier::BOLD),
        dim: Style::default().fg(Color::DarkGray),
        border: Style::default().fg(Color::DarkGray),
        hint: Style::default().fg(Color::Gray),
        status_normal: Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(Color::Yellow),
        help_title: Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        popup_bg: Color::Black,
        list_selected: Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    }
}

/// Light theme — for high-brightness terminals.
pub fn light() -> Theme {
    Theme {
        name: "light",
        background: Color::Reset,
        foreground: Color::Reset,
        bullet: Style::default().fg(Color::Gray),
        selected_bullet: Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(Color::White)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        ref_link: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        italic: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::ITALIC),
        strike: Style::default().add_modifier(Modifier::CROSSED_OUT),
        highlight: Style::default().bg(Color::Yellow).fg(Color::Black),
        code: Style::default().fg(Color::Magenta),
        todo_open: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        todo_done: Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(Color::Gray),
        property_value: Style::default().fg(Color::DarkGray),
        heading: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        dim: Style::default().fg(Color::Gray),
        border: Style::default().fg(Color::Gray),
        hint: Style::default().fg(Color::DarkGray),
        status_normal: Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(Color::White)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(Color::White)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(Color::Yellow),
        help_title: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        popup_bg: Color::White,
        list_selected: Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    }
}

/// Logseq light — Logseq's default light theme (Blueprint blues on
/// a white canvas).
pub fn logseq_light() -> Theme {
    theme_from_palette("logseq-light", &outl_theme::presets::logseq_light())
}

/// Dracula — popular dark palette.
pub fn dracula() -> Theme {
    theme_from_palette("dracula", &outl_theme::presets::dracula())
}

/// Solarized Dark — Ethan Schoonover's classic.
pub fn solarized_dark() -> Theme {
    theme_from_palette("solarized-dark", &outl_theme::presets::solarized_dark())
}

/// Nord — Arctic palette.
pub fn nord() -> Theme {
    theme_from_palette("nord", &outl_theme::presets::nord())
}

/// Monokai — Wimer Hazenberg's high-contrast palette.
pub fn monokai() -> Theme {
    theme_from_palette("monokai", &outl_theme::presets::monokai())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_preset_resolves() {
        for name in PRESETS {
            assert!(by_name(name).is_some(), "preset {name} should resolve");
        }
    }

    #[test]
    fn lookup_is_case_and_separator_insensitive() {
        assert!(by_name("Dracula").is_some());
        assert!(by_name("DRACULA").is_some());
        assert!(by_name("solarized_dark").is_some());
        assert!(by_name("Solarized Dark").is_some());
        assert!(by_name("default-dark").is_some());
    }

    #[test]
    fn unknown_preset_returns_none() {
        assert!(by_name("vampire").is_none());
        assert!(by_name("").is_none());
    }

    #[test]
    fn theme_name_matches_preset_id() {
        // Catches the "shipped Monokai but typo'd its name" class of bug.
        for name in PRESETS {
            let t = by_name(name).unwrap();
            assert_eq!(
                t.name, *name,
                "preset {name} resolved to a theme with name {}",
                t.name
            );
        }
    }
}
