//! `outl_theme::by_name` and `outl_tui::theme::by_name` must accept the
//! same names.
//!
//! The preset *list* has one owner now (`outl_theme::PRESETS`, re-exported
//! by the TUI), but the two resolvers are still separate functions: one
//! returns a `Palette` (hex, for the GUI clients), the other a ratatui
//! `Theme` (terminal styles). They cannot be merged — a terminal theme
//! like `default-dark` is built from ANSI `Color::Reset` / `Color::Cyan`,
//! which have no hex spelling.
//!
//! What *can* drift is which spellings each one accepts. Both carry a
//! hand-written alias table (`"dark"` → `default-dark`, `"solarized"` →
//! `solarized-dark`, …). Adding an alias to one and not the other means
//! `outl --theme dark` works in the terminal and silently falls back to
//! the default in the desktop app, or the reverse. That is invariant 13's
//! question — does this name mean the same thing everywhere it appears —
//! asked about the theme name rather than the colour token.

/// Every alias either resolver accepts. Written out rather than derived,
/// because the point is to fail when someone adds an arm to one `match`
/// and not the other: a list generated from the code under test would
/// happily agree with the bug.
const ALIASES: &[&str] = &["default", "dark", "logseq", "solarized", "gruvbox-dark"];

/// Spellings the normalizer is documented to fold: case, `_` and ` `
/// both becoming `-`. Both resolvers implement this independently.
const NORMALIZED_SPELLINGS: &[&str] = &[
    "Solarized Dark",
    "solarized_dark",
    "OUTL-LIGHT",
    "Logseq Light",
];

#[test]
fn both_resolvers_accept_every_preset_name() {
    for name in outl_theme::PRESETS {
        assert!(
            outl_theme::by_name(name).is_some(),
            "outl_theme::by_name rejects its own preset {name:?}"
        );
        assert!(
            outl_tui::theme::by_name(name).is_some(),
            "outl_tui::theme::by_name rejects preset {name:?} — the TUI \
             would fall back to the default while every other client \
             honours it"
        );
    }
}

#[test]
fn both_resolvers_accept_every_alias() {
    for name in ALIASES {
        assert!(
            outl_theme::by_name(name).is_some(),
            "outl_theme::by_name lost the alias {name:?}"
        );
        assert!(
            outl_tui::theme::by_name(name).is_some(),
            "outl_tui::theme::by_name lost the alias {name:?}"
        );
    }
}

#[test]
fn both_resolvers_normalize_case_and_separators_the_same_way() {
    for name in NORMALIZED_SPELLINGS {
        assert!(
            outl_theme::by_name(name).is_some(),
            "outl_theme::by_name stopped normalizing {name:?}"
        );
        assert!(
            outl_tui::theme::by_name(name).is_some(),
            "outl_tui::theme::by_name stopped normalizing {name:?}"
        );
    }
}

#[test]
fn both_resolvers_reject_the_same_unknown_name() {
    // A resolver that quietly resolves an unknown name to the default
    // makes a typo in `.outl/config.toml` undiagnosable.
    assert!(outl_theme::by_name("nope-not-a-theme").is_none());
    assert!(outl_tui::theme::by_name("nope-not-a-theme").is_none());
}

/// The resolved TUI theme must carry the name it was asked for, so
/// `outl theme show <name>` cannot print a different theme's label than
/// the one the user typed.
#[test]
fn the_tui_theme_reports_the_canonical_name() {
    for name in outl_theme::PRESETS {
        let theme = outl_tui::theme::by_name(name).expect("preset resolves");
        assert_eq!(
            &theme.name, name,
            "preset {name:?} resolved to a theme labelled {:?}",
            theme.name
        );
    }
}
