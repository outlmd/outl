//! The theme file format: one `.toml` per family, `[dark]` and `[light]`
//! inside it, names derived from the filename and the section.
//!
//! This file owns the format's contract, which is the same contract for a
//! built-in theme (embedded with `include_str!`) and for a user's theme in
//! `~/.config/outl/themes/`. One format, one parser, one `extends`
//! resolver — a second definition of "what a theme is" is what this test
//! exists to prevent.

use outl_theme::spec::{parse_family, ThemeError};
use outl_theme::Palette;

/// Every `Palette` field set to one hex, so a test can build a complete
/// section without listing 45 names by hand. The field *names* come from
/// `fields()`; the values are the test's own.
fn full_section(hex: &str) -> String {
    section_without("", hex)
}

/// A complete section minus one named field, so a test can prove the
/// error names *that* field rather than whichever one happens to be
/// first in declaration order.
fn section_without(skip: &str, hex: &str) -> String {
    outl_theme::default()
        .fields()
        .iter()
        .filter(|(k, _)| *k != skip)
        .map(|(k, _)| format!("{k} = \"{hex}\""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn no_bases(_: &str) -> Option<Palette> {
    None
}

#[test]
fn a_dark_section_is_named_after_the_file() {
    let src = format!("[dark]\n{}", full_section("#111111"));
    let entries = parse_family("gruvbox", &src, &no_bases).expect("parses");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].canonical, "gruvbox");
    assert_eq!(entries[0].palette.name, "gruvbox");
    assert_eq!(entries[0].palette.bg, "#111111");
}

#[test]
fn a_dark_section_also_answers_to_the_dark_suffix() {
    let src = format!("[dark]\n{}", full_section("#111111"));
    let entries = parse_family("gruvbox", &src, &no_bases).expect("parses");

    assert!(
        entries[0].aliases.contains(&"gruvbox-dark".to_string()),
        "the `-dark` suffix is derived, never written: {:?}",
        entries[0].aliases
    );
}

#[test]
fn a_light_section_is_named_with_the_light_suffix() {
    let src = format!(
        "[dark]\n{}\n[light]\nbg = \"#ffffff\"\n",
        full_section("#111111")
    );
    let entries = parse_family("gruvbox", &src, &no_bases).expect("parses");

    let light = entries
        .iter()
        .find(|e| e.canonical == "gruvbox-light")
        .expect("light side resolves as <family>-light");
    assert_eq!(light.palette.name, "gruvbox-light");
}

#[test]
fn a_light_section_inherits_every_field_it_does_not_set() {
    let src = format!(
        "[dark]\n{}\n[light]\nbg = \"#ffffff\"\nfg = \"#222222\"\n",
        full_section("#111111")
    );
    let entries = parse_family("gruvbox", &src, &no_bases).expect("parses");
    let light = &entries
        .iter()
        .find(|e| e.canonical == "gruvbox-light")
        .unwrap()
        .palette;

    assert_eq!(light.bg, "#ffffff", "its own value wins");
    assert_eq!(light.fg, "#222222");
    assert_eq!(light.accent, "#111111", "inherited from [dark]");
}

#[test]
fn a_family_with_only_a_light_side_answers_to_the_bare_name() {
    let src = format!("[light]\n{}", full_section("#fefefe"));
    let entries = parse_family("logseq", &src, &no_bases).expect("parses");

    assert_eq!(entries[0].canonical, "logseq-light");
    assert!(
        entries[0].aliases.contains(&"logseq".to_string()),
        "the bare family name resolves to the only side there is: {:?}",
        entries[0].aliases
    );
}

#[test]
fn a_missing_field_with_nothing_to_inherit_names_the_field() {
    let src = format!("[dark]\n{}", section_without("fg", "#111111"));
    let err = parse_family("half", &src, &no_bases).expect_err("incomplete");

    let msg = err.to_string();
    assert!(
        msg.contains("`fg`"),
        "the error has to name the field that is missing, got: {msg}"
    );
}

#[test]
fn an_unknown_field_is_a_parse_error_not_a_silent_drop() {
    let src = format!(
        "[dark]\n{}\nbakground = \"#111111\"\n",
        full_section("#111111")
    );
    let err = parse_family("typo", &src, &no_bases).expect_err("unknown key");

    assert!(
        err.to_string().contains("bakground"),
        "a typo'd field must name itself, got: {err}"
    );
}

#[test]
fn a_non_hex_value_is_rejected_at_parse_time() {
    let src = format!(
        "[dark]\n{}\naccent = \"rebeccapurple\"\n",
        full_section("#111111")
    );
    let err = parse_family("css", &src, &no_bases).expect_err("not hex");

    let msg = err.to_string();
    assert!(
        msg.contains("accent") && msg.contains("rebeccapurple"),
        "the error names the field and the bad value, got: {msg}"
    );
}

#[test]
fn extends_inherits_from_another_theme() {
    let base: Palette = {
        let src = format!("[dark]\n{}", full_section("#111111"));
        parse_family("base", &src, &no_bases).unwrap()[0]
            .palette
            .clone()
    };
    let lookup = |name: &str| (name == "base").then(|| base.clone());

    let src = "[dark]\nextends = \"base\"\naccent = \"#ff6ac1\"\n";
    let entries = parse_family("mine", src, &lookup).expect("parses");

    assert_eq!(entries[0].palette.accent, "#ff6ac1", "own value wins");
    assert_eq!(entries[0].palette.bg, "#111111", "rest comes from base");
    assert_eq!(entries[0].palette.name, "mine", "name is never inherited");
}

#[test]
fn extends_pointing_at_nothing_names_what_it_wanted() {
    let src = "[dark]\nextends = \"nope\"\naccent = \"#ff6ac1\"\n";
    let err = parse_family("mine", src, &no_bases).expect_err("unknown base");

    assert!(
        err.to_string().contains("nope"),
        "the error names the base it could not find, got: {err}"
    );
}

#[test]
fn a_light_side_can_extend_a_foreign_theme_instead_of_its_sibling() {
    let base: Palette = {
        let src = format!("[dark]\n{}", full_section("#eeeeee"));
        parse_family("paper", &src, &no_bases).unwrap()[0]
            .palette
            .clone()
    };
    let lookup = |name: &str| (name == "paper").then(|| base.clone());

    let src = format!(
        "[dark]\n{}\n[light]\nextends = \"paper\"\naccent = \"#6d28d9\"\n",
        full_section("#111111")
    );
    let entries = parse_family("mine", &src, &lookup).expect("parses");
    let light = &entries
        .iter()
        .find(|e| e.canonical == "mine-light")
        .unwrap()
        .palette;

    assert_eq!(
        light.bg, "#eeeeee",
        "an explicit extends replaces the sibling default"
    );
    assert_eq!(light.accent, "#6d28d9");
}

#[test]
fn a_file_with_no_sides_is_an_error() {
    let err = parse_family("empty", "", &no_bases).expect_err("no sides");
    assert!(
        matches!(err, ThemeError::NoSides { .. }),
        "got {err:?}, want NoSides"
    );
}

#[test]
fn the_name_field_is_not_part_of_the_format() {
    let src = format!(
        "[dark]\n{}\nname = \"something-else\"\n",
        full_section("#111111")
    );
    let err = parse_family("gruvbox", &src, &no_bases).expect_err("name is derived");

    assert!(
        err.to_string().contains("name"),
        "`name` comes from the filename; writing it is an error, got: {err}"
    );
}
