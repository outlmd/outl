//! Arm-pattern snapshots of `outl-tui`'s Normal-mode key handling.
//!
//! The TUI half of `tui_catalog_divergence.rs` — see that file for
//! why the divergence list exists at all.
//!
//! `outl-shortcuts` cannot depend on `outl-tui` (that is the
//! dependency edge, pointing the other way), so the TUI half is
//! pinned by reading its source. Coarse on purpose: it captures the
//! *arm patterns* of the two `match`es in `handle_normal_key`, not
//! their bodies, so ordinary edits inside a handler don't fire it and
//! a re-keyed, added or deleted arm does.

/// Chord-pair arms — the `match (pending, key.code)` block.
const PAIR_ARMS: &[&str] = &[
    r#"('d', KeyCode::Char('d'))"#,
    r#"('g', KeyCode::Char('j'))"#,
    r#"('g', KeyCode::Char('x'))"#,
    r#"('g', KeyCode::Char('g'))"#,
    r#"('g', KeyCode::Char('p'))"#,
    r#"('g', KeyCode::Char('P'))"#,
    r#"('g', KeyCode::Char('d'))"#,
    r#"('g', KeyCode::Char('r'))"#,
    r#"('g', KeyCode::Char('R'))"#,
    r#"('g', KeyCode::Char('s'))"#,
    r#"('g', KeyCode::Char('n'))"#,
    r#"('y', KeyCode::Char('y'))"#,
    r#"('y', KeyCode::Char('r'))"#,
    r#"('q', KeyCode::Char('q'))"#,
    r#"('Z', KeyCode::Char('Z'))"#,
    r#"('g', KeyCode::Char('v'))"#,
    r#"('z', KeyCode::Char('R'))"#,
    r#"('z', KeyCode::Char('M'))"#,
    r#"('z', KeyCode::Char('z'))"#,
    r#"('z', KeyCode::Char('i'))"#,
    r#"('z', KeyCode::Char('o'))"#,
];

/// Bare-key arms — the function-level `match key.code` block.
const BARE_ARMS: &[&str] = &[
    r#"KeyCode::Char('q')"#,
    r#"KeyCode::Char('Z')"#,
    r#"KeyCode::Char('?')"#,
    r#"KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('t')"#,
    r#"KeyCode::Char('[')"#,
    r#"KeyCode::Char(']')"#,
    r#"KeyCode::Char('g')"#,
    r#"KeyCode::Char('z')"#,
    r#"KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('d')"#,
    r#"KeyCode::Char('y')"#,
    r#"KeyCode::Char('c')"#,
    r#"KeyCode::Char('p') if key.modifiers.is_empty()"#,
    r#"KeyCode::Char('P') if key.modifiers == KeyModifiers::SHIFT || key.modifiers.is_empty()"#,
    r#"KeyCode::Tab"#,
    r#"KeyCode::BackTab"#,
    r#"KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Enter if !app.try_open_under_cursor()?"#,
    r#"KeyCode::Enter"#,
    r#"KeyCode::Char('i')"#,
    r#"KeyCode::Char('I')"#,
    r#"KeyCode::Char('a')"#,
    r#"KeyCode::Char('A')"#,
    r#"KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('o')"#,
    r#"KeyCode::Char('O')"#,
    r#"KeyCode::Char('x')"#,
    r#"KeyCode::Char('X')"#,
    r#"KeyCode::Char('D')"#,
    r#"KeyCode::Char('C')"#,
    r#"KeyCode::Char('S')"#,
    r#"KeyCode::Char('s')"#,
    r#"KeyCode::Char('r') if key.modifiers.is_empty()"#,
    r#"KeyCode::Char('f') if key.modifiers.is_empty()"#,
    r#"KeyCode::Char('F')"#,
    r#"KeyCode::Char('~')"#,
    r#"KeyCode::Char('Y')"#,
    r#"KeyCode::Char('e') if key.modifiers.is_empty()"#,
    r#"KeyCode::Char('*')"#,
    r#"KeyCode::Char('#')"#,
    r#"KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT)"#,
    r#"KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT)"#,
    r#"KeyCode::Down | KeyCode::Char('j')"#,
    r#"KeyCode::Up | KeyCode::Char('k')"#,
    r#"KeyCode::PageDown"#,
    r#"KeyCode::PageUp"#,
    r#"KeyCode::Char('G')"#,
    r#"KeyCode::Left | KeyCode::Char('h')"#,
    r#"KeyCode::Right | KeyCode::Char('l')"#,
    r#"KeyCode::Char('0') | KeyCode::Home"#,
    r#"KeyCode::Char('$') | KeyCode::End"#,
    r#"KeyCode::Char('b' | 'B') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('e' | 'E') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('w')"#,
    r#"KeyCode::Char('b')"#,
    r#"KeyCode::Char('K')"#,
    r#"KeyCode::Char('J')"#,
    r#"KeyCode::Char('u')"#,
    r#"KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL)"#,
    r#"KeyCode::Char('/')"#,
    r#"KeyCode::Char(':')"#,
    r#"KeyCode::Char('n')"#,
    r#"KeyCode::Char('N')"#,
    r#"KeyCode::Char('V')"#,
];

fn normal_rs() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../outl-tui/src/input/normal.rs"
    );
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    // Comments carry `KeyCode::` references (the arms are heavily
    // annotated), so strip them before matching patterns.
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => l[..i].to_string(),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Arm patterns inside one `match` block, identified by the exact
/// indentation its arms sit at.
fn arms_in(region: &str, indent: usize, starts_with: &str) -> Vec<String> {
    let pad = " ".repeat(indent);
    region
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix(&pad)?;
            if rest.starts_with(' ') || !rest.starts_with(starts_with) {
                return None;
            }
            let pat = rest.rsplit_once("=>")?.0;
            Some(squash(pat))
        })
        .collect()
}

#[test]
fn the_tui_normal_key_arms_have_not_moved() {
    let src = normal_rs();

    let bare_at = src
        .rfind("\n    match key.code {")
        .expect("handle_normal_key's function-level `match key.code` moved or was renamed");
    let pair_at = src
        .find("match (pending, key.code) {")
        .expect("handle_normal_key's chord-pair `match` moved or was renamed");

    let pairs = arms_in(&src[pair_at..bare_at], 12, "(");
    let bares = arms_in(&src[bare_at..], 8, "KeyCode::");

    // Two assertions rather than one so the failure names which half
    // moved; the message is the same either way because the required
    // response is the same.
    const WHY: &str = "\n\noutl-tui's Normal-mode key arms changed. This file records 17 chords \
        where the TUI and this catalog already disagree (6 of them destructive), and that list \
        is only true for the arms it was written against. Re-check the chords you touched \
        against KNOWN_DIVERGENCES / AGREED above, then update this snapshot in the same commit. \
        Do not update the snapshot first.";

    assert_eq!(
        pairs,
        PAIR_ARMS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "chord-pair arms{WHY}",
    );
    assert_eq!(
        bares,
        BARE_ARMS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "bare-key arms{WHY}",
    );
}

#[test]
fn the_quit_intercept_still_sits_above_the_normal_handler() {
    // `Ctrl+C` is the one divergence whose TUI half is not in
    // `normal.rs` at all: `runtime.rs` returns from the event loop
    // before `handle_normal_key` is called, so the catalog's
    // `Normal` → `CopyBlock` row can never fire on the TUI. Pinned
    // separately because the snapshot above cannot see it.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../outl-tui/src/runtime.rs");
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    assert!(
        squash(&src).contains(squash(
            "if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')"
        ).as_str()),
        "the Ctrl+C quit intercept in outl-tui/src/runtime.rs moved — the `Ctrl+c` row in \
         KNOWN_DIVERGENCES describes it, so re-check that row",
    );
}
