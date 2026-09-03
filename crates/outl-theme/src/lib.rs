//! # outl-theme
//!
//! Shared palette definitions for every outl client that paints
//! something — `outl-tui` (terminal, via `ratatui::Color`) and
//! `outl-desktop` (Tauri + Solid, via CSS custom properties).
//!
//! The crate is intentionally **dependency-light**: no `ratatui`,
//! no CSS engine, just `serde` so the palette can ride the Tauri
//! wire as JSON.
//!
//! A [`Palette`] holds named hex colors for every semantic surface
//! the outliner paints (background, accent, ref-link, todo-open,
//! …). Each client converts those hex strings to whatever its
//! renderer wants:
//!
//! - **TUI** maps `#a78bfa` → `ratatui::style::Color::Rgb(167, 139, 250)`
//!   and re-applies the modifiers it always wanted (`BOLD` on
//!   `bold`, `UNDERLINED` on `ref_link`, `ITALIC` on `italic`, …) —
//!   see `outl-tui/src/theme.rs::Theme::from_palette`.
//! - **Desktop** writes each field as a CSS custom property on
//!   `<html>` (`--color-outl-accent: #a78bfa`) which Tailwind class
//!   utilities reference (`bg-(--color-outl-accent)`).
//!
//! ## Presets
//!
//! Nine built-in palettes, named identically to the TUI presets
//! that shipped before this crate existed. The TUI's render path is
//! unchanged; it now derives its `Theme` from these. Adding a
//! preset means: extend `presets`, add a name to
//! [`PRESETS`], and add a match arm in [`by_name`].

pub mod palette;
pub mod presets;

pub use palette::Palette;

/// List of built-in preset names, in the order surfaced to users
/// in pickers (CLI, settings modal, command palette).
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

/// Resolve a preset by name. Accepts the canonical name or any of
/// the documented aliases; case- and separator-insensitive
/// (`"Solarized Dark"` and `"solarized_dark"` both resolve to
/// `solarized-dark`).
///
/// Returns `None` when nothing matches so callers can fall back to
/// [`default`] without panicking on malformed user input.
pub fn by_name(name: &str) -> Option<Palette> {
    let norm: String = name
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            '_' | ' ' => '-',
            _ => c,
        })
        .collect();
    match norm.as_str() {
        "outl" | "default" => Some(presets::outl()),
        "outl-light" => Some(presets::outl_light()),
        "default-dark" | "dark" => Some(presets::default_dark()),
        "light" => Some(presets::light()),
        "logseq-light" | "logseq" => Some(presets::logseq_light()),
        "dracula" => Some(presets::dracula()),
        "solarized-dark" | "solarized" => Some(presets::solarized_dark()),
        "nord" => Some(presets::nord()),
        "monokai" => Some(presets::monokai()),
        _ => None,
    }
}

/// Default palette returned when nothing is configured. Mirrors the
/// TUI's `theme::default_theme()` choice — `outl` is the brand
/// palette every other client picks up too.
pub fn default() -> Palette {
    presets::outl()
}

/// Iterator over every built-in palette, in [`PRESETS`] order.
/// Useful for pickers that need to list everything without naming
/// each helper.
pub fn all() -> Vec<Palette> {
    PRESETS.iter().filter_map(|name| by_name(name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outl_light_reads_as_light() {
        // `outl-light` is the light half of mobile's default pair
        // (`App.tsx`: `preset: "outl-light", presetDark: "outl"`).
        // `outl doctor`'s `check_theme_pair` warns when a pair's
        // light slot resolves to a dark palette — this pins the
        // preset never regresses into tripping that check.
        assert!(presets::outl_light().is_light());
        assert!(
            !presets::outl().is_light(),
            "outl must stay the dark side of the pair"
        );
    }

    #[test]
    fn every_listed_preset_resolves() {
        for name in PRESETS {
            assert!(
                by_name(name).is_some(),
                "PRESETS lists {name} but by_name returns None"
            );
        }
    }

    #[test]
    fn name_matches_listed_preset() {
        for name in PRESETS {
            let p = by_name(name).unwrap();
            assert_eq!(
                p.name.as_str(),
                *name,
                "palette name drifted from PRESETS entry"
            );
        }
    }

    #[test]
    fn by_name_is_case_and_separator_insensitive() {
        assert_eq!(by_name("DRACULA").unwrap().name.as_str(), "dracula");
        assert_eq!(
            by_name("Solarized Dark").unwrap().name.as_str(),
            "solarized-dark"
        );
        assert_eq!(
            by_name("solarized_dark").unwrap().name.as_str(),
            "solarized-dark"
        );
    }

    #[test]
    fn unknown_palette_returns_none() {
        assert!(by_name("not-a-real-theme").is_none());
    }

    #[test]
    fn every_palette_field_is_hex() {
        // The whole point of this crate is that every client gets a
        // hex string per field. A missed field would surface as an
        // empty or non-`#` value when the palette ships across the
        // Tauri wire. Catch it once, not at render-time.
        for p in all() {
            for (field, value) in p.fields() {
                assert!(
                    value.starts_with('#') && (value.len() == 7 || value.len() == 9),
                    "palette {} field {field} is not hex: {value:?}",
                    p.name
                );
            }
        }
    }
}
