//! # outl-theme
//!
//! Shared palette definitions for every outl client that paints
//! something — `outl-tui` (terminal, via `ratatui::Color`) and
//! `outl-desktop` / `outl-mobile` (Tauri + Solid, via CSS custom
//! properties).
//!
//! A [`Palette`] holds named hex colors for every semantic surface
//! the outliner paints (background, accent, ref-link, todo-open,
//! …). Each client converts those hex strings to whatever its
//! renderer wants:
//!
//! - **TUI** maps `#a78bfa` → `ratatui::style::Color::Rgb(167, 139, 250)`
//!   and re-applies the modifiers it always wanted (`BOLD` on
//!   `bold`, `UNDERLINED` on `ref_link`, `ITALIC` on `italic`, …) —
//!   see `outl-tui/src/theme.rs::theme_from_palette`.
//! - **Desktop / mobile** write each field as a CSS custom property on
//!   `<html>` (`--color-outl-accent: #a78bfa`) which Tailwind class
//!   utilities reference (`bg-(--color-outl-accent)`).
//!
//! ## Where a theme comes from
//!
//! One `.toml` per family in `themes/`, with `[dark]` and `[light]`
//! inside it. Built-ins are `include_str!`'d into the binary, because
//! outl ships as a binary and a built-in theme cannot be a file the user
//! has to have. A user's own theme is the **same format**, read from a
//! directory the client passes in ([`registry::Registry::with_user_dir`]),
//! so `extends = "dracula"` in `~/.config/outl/themes/mine.toml` resolves
//! against the built-in it names.
//!
//! [`spec`] owns the format, [`registry`] owns the catalog. The functions
//! here are thin wrappers over [`registry::Registry::builtin`], kept
//! because most callers only ever want a built-in by name.

pub mod palette;
pub mod registry;
pub mod spec;

pub use palette::Palette;
pub use registry::Registry;

/// Canonical preset names, in the order surfaced to users in pickers
/// (CLI, settings modal, command palette).
///
/// Derived from the theme files rather than declared: the name of each
/// side comes from its filename and its section, so this list cannot
/// disagree with what [`by_name`] resolves. It was a hand-written `const`
/// array until the files became the source, and keeping both was two
/// owners of one fact.
pub fn preset_names() -> Vec<&'static str> {
    Registry::builtin().names()
}

/// Resolve a built-in preset by name.
///
/// Accepts the canonical name, the derived `-dark` / `-light` suffix, the
/// bare family name, and the legacy spellings kept for older configs.
/// Case- and separator-insensitive (`"Solarized Dark"` and
/// `"solarized_dark"` both resolve).
///
/// Returns `None` when nothing matches so callers can fall back to
/// [`default`] without panicking on malformed user input.
///
/// This sees **built-ins only**. A client that also honours the user's
/// `themes/` directory builds a [`Registry::with_user_dir`] and asks that.
pub fn by_name(name: &str) -> Option<Palette> {
    Registry::builtin().by_name(name).cloned()
}

/// Default palette returned when nothing is configured — `outl`, the
/// brand palette every client picks up.
pub fn default() -> Palette {
    by_name("outl").expect("the brand preset is built in")
}

/// Every built-in palette, in [`preset_names`] order. Useful for pickers
/// that need to list everything without naming each theme.
pub fn all() -> Vec<Palette> {
    Registry::builtin().all().into_iter().cloned().collect()
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
        assert!(by_name("outl-light").unwrap().is_light());
        assert!(
            !by_name("outl").unwrap().is_light(),
            "outl must stay the dark side of the pair"
        );
    }

    #[test]
    fn every_listed_preset_resolves() {
        for name in preset_names() {
            assert!(
                by_name(name).is_some(),
                "preset_names lists {name} but by_name returns None"
            );
        }
    }

    #[test]
    fn name_matches_listed_preset() {
        for name in preset_names() {
            let p = by_name(name).unwrap();
            assert_eq!(
                p.name.as_str(),
                name,
                "palette name drifted from its preset_names entry"
            );
        }
    }

    #[test]
    fn by_name_is_case_and_separator_insensitive() {
        assert_eq!(by_name("DRACULA").unwrap().name.as_str(), "dracula");
        assert_eq!(
            by_name("Solarized Dark").unwrap().name.as_str(),
            "solarized"
        );
        assert_eq!(
            by_name("solarized_dark").unwrap().name.as_str(),
            "solarized"
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
        //
        // `spec` already refuses a non-hex value at parse time, so this
        // is the second line rather than the first — it also covers a
        // palette built any other way.
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
