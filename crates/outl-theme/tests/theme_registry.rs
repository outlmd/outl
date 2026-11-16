//! The registry: the built-in themes embedded in the binary, plus the
//! user's own from a directory *passed in as an argument*.
//!
//! The directory is a parameter, never resolved from `$HOME` in here, and
//! that is deliberate. `XDG_CONFIG_HOME` is process-wide, and a crate that
//! reaches for it by itself is a crate whose tests write into the
//! developer's real `~/.config/outl` — which already happened in 0.11.0
//! and made three doctor tests flaky (invariant 9: state that leaves a
//! boundary has to say how a test gets its own copy). The client resolves
//! the path; this crate reads what it is handed.

use outl_theme::registry::Registry;

/// Canonical names, in the order a picker shows them.
const EXPECTED: &[&str] = &[
    "outl",
    "outl-light",
    "ansi",
    "ansi-light",
    "logseq-light",
    "dracula",
    "solarized",
    "nord",
    "monokai",
    "gruvbox",
    "gruvbox-light",
];

#[test]
fn the_builtin_registry_lists_every_theme_in_picker_order() {
    assert_eq!(Registry::builtin().names(), EXPECTED);
}

#[test]
fn every_builtin_name_resolves_to_a_palette_that_owns_that_name() {
    for name in EXPECTED {
        let p = Registry::builtin()
            .by_name(name)
            .unwrap_or_else(|| panic!("{name} is listed but does not resolve"));
        assert_eq!(p.name.as_str(), *name, "palette name drifted from its key");
    }
}

#[test]
fn the_dark_suffix_is_derived_for_every_family_with_a_dark_side() {
    for (suffixed, canonical) in [
        ("outl-dark", "outl"),
        ("gruvbox-dark", "gruvbox"),
        ("solarized-dark", "solarized"),
        ("dracula-dark", "dracula"),
        ("ansi-dark", "ansi"),
    ] {
        let p = Registry::builtin()
            .by_name(suffixed)
            .unwrap_or_else(|| panic!("{suffixed} should resolve by derivation"));
        assert_eq!(p.name.as_str(), canonical);
    }
}

#[test]
fn a_light_only_family_resolves_by_its_bare_name() {
    let p = Registry::builtin()
        .by_name("logseq")
        .expect("logseq resolves");
    assert_eq!(p.name.as_str(), "logseq-light");
}

/// The names the derivation does not produce, kept so no existing
/// `config.toml` breaks. This list may only ever get shorter.
#[test]
fn every_legacy_name_still_resolves() {
    for (legacy, target) in [
        ("default-dark", "ansi"),
        ("dark", "ansi"),
        ("light", "ansi-light"),
        ("default", "outl"),
    ] {
        let p = Registry::builtin()
            .by_name(legacy)
            .unwrap_or_else(|| panic!("legacy name {legacy} stopped resolving"));
        assert_eq!(
            p.name.as_str(),
            target,
            "legacy name {legacy} should still reach {target}"
        );
    }
}

#[test]
fn lookup_is_case_and_separator_insensitive() {
    assert_eq!(
        Registry::builtin().by_name("Solarized Dark").unwrap().name,
        "solarized"
    );
    assert_eq!(
        Registry::builtin().by_name("GRUVBOX_LIGHT").unwrap().name,
        "gruvbox-light"
    );
}

#[test]
fn an_unknown_name_resolves_to_nothing() {
    assert!(Registry::builtin().by_name("not-a-real-theme").is_none());
}

#[test]
fn a_user_theme_extends_a_builtin_without_recompiling_anything() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("mine.toml"),
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n",
    )
    .expect("write theme");

    let (reg, errors) = Registry::with_user_dir(dir.path());
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");

    let mine = reg.by_name("mine").expect("the user's theme resolves");
    assert_eq!(mine.accent, "#ff6ac1", "its own value");
    assert_eq!(
        mine.bg,
        Registry::builtin().by_name("outl").unwrap().bg,
        "everything else comes from the built-in it extends"
    );
    assert!(
        reg.names().contains(&"mine"),
        "a user theme shows up in the picker: {:?}",
        reg.names()
    );
}

#[test]
fn a_user_theme_may_carry_both_sides_like_a_builtin() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("mine.toml"),
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n\
         [light]\nextends = \"outl-light\"\naccent = \"#6d28d9\"\n",
    )
    .expect("write theme");

    let (reg, errors) = Registry::with_user_dir(dir.path());
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(reg.by_name("mine").unwrap().accent, "#ff6ac1");
    assert_eq!(reg.by_name("mine-light").unwrap().accent, "#6d28d9");
}

#[test]
fn a_user_theme_overrides_a_builtin_of_the_same_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("dracula.toml"),
        "[dark]\nextends = \"dracula\"\naccent = \"#000001\"\n",
    )
    .expect("write theme");

    let (reg, errors) = Registry::with_user_dir(dir.path());
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(
        reg.by_name("dracula").unwrap().accent,
        "#000001",
        "the user's file wins over the built-in it shadows"
    );
    assert_eq!(
        reg.names().iter().filter(|n| **n == "dracula").count(),
        1,
        "and it replaces the entry rather than adding a second one"
    );
}

#[test]
fn a_broken_user_theme_is_reported_and_the_rest_still_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("broken.toml"),
        "[dark]\naccent = \"nope\"\n",
    )
    .expect("write");
    std::fs::write(
        dir.path().join("fine.toml"),
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n",
    )
    .expect("write");

    let (reg, errors) = Registry::with_user_dir(dir.path());

    assert_eq!(errors.len(), 1, "exactly the broken one: {errors:?}");
    assert!(
        errors[0].to_string().contains("broken"),
        "the error names the theme that failed: {}",
        errors[0]
    );
    assert!(
        reg.by_name("broken").is_none(),
        "a theme that failed to load must not resolve to a half-painted palette"
    );
    assert!(
        reg.by_name("fine").is_some(),
        "one bad file does not take the others down"
    );
}

#[test]
fn a_user_theme_extending_a_cycle_is_reported_not_hung() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.toml"), "[dark]\nextends = \"b\"\n").expect("write");
    std::fs::write(dir.path().join("b.toml"), "[dark]\nextends = \"a\"\n").expect("write");

    let (reg, errors) = Registry::with_user_dir(dir.path());

    assert_eq!(errors.len(), 2, "both sides of the cycle: {errors:?}");
    assert!(reg.by_name("a").is_none());
    assert!(reg.by_name("b").is_none());
}

#[test]
fn a_missing_user_dir_is_not_an_error() {
    let (reg, errors) =
        Registry::with_user_dir(std::path::Path::new("/nonexistent/outl-theme-test/themes"));
    assert!(errors.is_empty(), "no themes directory is the normal case");
    assert_eq!(reg.names(), EXPECTED, "built-ins are unaffected");
}

#[test]
fn a_non_toml_file_in_the_theme_dir_is_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("README.md"), "not a theme\n").expect("write");
    let (reg, errors) = Registry::with_user_dir(dir.path());
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(reg.names(), EXPECTED);
}

#[test]
fn a_theme_file_that_cannot_be_read_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A directory named `*.toml` fails `read_to_string` for every user,
    // including root — unlike `chmod 000`, which CI as root ignores.
    std::fs::create_dir(dir.path().join("weird.toml")).expect("mkdir");

    let (reg, errors) = Registry::with_user_dir(dir.path());

    assert_eq!(errors.len(), 1, "{errors:?}");
    let msg = errors[0].to_string();
    assert!(msg.contains("weird"), "names the file: {msg}");
    assert!(
        !msg.contains("[dark]"),
        "an unreadable file is not a file missing a section — that wording \
         sends the user looking for a typo in bytes nobody could read: {msg}"
    );
    assert!(reg.by_name("weird").is_none());
}
