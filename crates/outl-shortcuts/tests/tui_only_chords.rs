//! The reverse direction of `tui_catalog_divergence.rs`.
//!
//! That file walks catalog → TUI, so it cannot see a chord only the
//! TUI has. 31 of those ship today, which means `docs/shortcuts.md`'s
//! generated halves and the desktop's help overlay are blind to them,
//! and `outl-shortcuts`'s own rule is "No client hardcodes a
//! `(chord, action)` pair" ([#441](https://github.com/outlmd/outl/issues/441)).
//!
//! Its own file rather than a third section of the divergence test:
//! that one was 3 lines from the 600-line ratchet, and this is a
//! separate question with a separate list.

use outl_shortcuts::{lookup, ChordSequence, Mode};

fn seq(s: &str) -> ChordSequence {
    ChordSequence::parse(s).unwrap_or_else(|| panic!("unparseable chord in this test: {s:?}"))
}

/// Chords `outl-tui` binds in Normal mode that the catalog does not
/// carry at all.
///
/// The reverse direction of [`every_terminal_reachable_binding_has_a_verdict`],
/// which walks catalog → TUI and therefore cannot see a binding that
/// only the TUI has. 28 of them ship today, so
/// `docs/shortcuts.md`'s generated halves and the desktop's help
/// overlay are blind to them, and `outl-shortcuts`'s own rule is "No
/// client hardcodes a `(chord, action)` pair".
///
/// Declared rather than derived, for the same reason
/// `tui_normal_arms` is: this crate cannot depend on `outl-tui`.
/// Adding one of these to `defaults.rs` fails
/// [`every_tui_only_chord_is_absent_from_the_catalog`], which is the
/// point — the list is what makes the next person delete the row they
/// just made redundant.
///
/// Migrating these into the catalog is [#293](https://github.com/outlmd/outl/issues/293),
/// not this gate: each one needs an `Action` variant, and `support()`
/// is an exhaustive `match` over `Action`, so every addition is also
/// three per-client verdicts.
const TUI_ONLY: &[(&str, &str)] = &[
    ("g g", "jump to the first block"),
    ("y y", "yank the current block"),
    ("Ctrl+d", "half page down"),
    ("Ctrl+u", "half page up"),
    ("p", "paste the OS clipboard, with formatting"),
    ("Shift+p", "paste the OS clipboard, plain"),
    ("Ctrl+o", "flip the backlinks sort order"),
    ("Alt+Up", "move the block up"),
    ("Alt+Down", "move the block down"),
    ("PageDown", "page down"),
    ("PageUp", "page up"),
    ("Shift+g", "jump to the last block"),
    ("h", "cursor left inside the block"),
    ("Left", "cursor left inside the block"),
    ("l", "cursor right inside the block"),
    ("Right", "cursor right inside the block"),
    // `0` and `Home` share one TUI arm, but only `0` is absent here:
    // `Home` is in the catalog as `OpenToday` and is already a
    // KNOWN_DIVERGENCES row above. This gate caught that on its first
    // run, which is the direction it exists for.
    ("0", "cursor to the start of the line"),
    ("$", "cursor to the end of the line"),
    ("End", "cursor to the end of the line"),
    ("Ctrl+b", "toggle the inline backlinks panel"),
    ("Ctrl+e", "toggle the sidebar"),
    ("w", "cursor one word right"),
    ("b", "cursor one word left"),
    ("Shift+k", "move the block up"),
    ("Shift+j", "move the block down"),
    ("/", "open the slash menu"),
    ("n", "next search hit"),
    ("Shift+n", "previous search hit"),
    ("Shift+v", "enter Visual mode"),
    ("Ctrl+s", "force a save"),
    ("Ctrl+l", "reload the workspace from disk"),
];

/// Every `TUI_ONLY` row is genuinely missing from the catalog.
///
/// Fails in the direction that matters: somebody adds one of these to
/// `defaults.rs` (closing part of #293) and this says so, instead of
/// leaving a stale list claiming the gap is still open.
#[test]
fn every_tui_only_chord_is_absent_from_the_catalog() {
    let mut now_in_catalog = Vec::new();
    for (chord, what) in TUI_ONLY {
        let parsed = seq(chord);
        for mode in [Mode::Normal, Mode::Global] {
            if let Some(action) = lookup(mode, &parsed) {
                now_in_catalog.push(format!(
                    "{chord:?} ({what}) is now {mode:?} → {action:?} in the catalog"
                ));
            }
        }
    }
    assert!(
        now_in_catalog.is_empty(),
        "these chords are in the catalog now, so TUI_ONLY is stale. Drop each row \
         and make sure the TUI dispatches it through `lookup`:\n  {}",
        now_in_catalog.join("\n  "),
    );
}

/// `TUI_ONLY` spellings parse, and none is listed twice.
///
/// A typo here reads as "the catalog does not have this", which is
/// true of any unparseable string, so the row would pass the gate
/// above while documenting nothing.
#[test]
fn tui_only_rows_parse_and_are_unique() {
    let mut seen: Vec<ChordSequence> = Vec::new();
    for (chord, _) in TUI_ONLY {
        let parsed = ChordSequence::parse(chord)
            .unwrap_or_else(|| panic!("TUI_ONLY has an unparseable chord: {chord:?}"));
        assert!(!seen.contains(&parsed), "TUI_ONLY lists {chord:?} twice");
        seen.push(parsed);
    }
}

/// The count in the prose matches the list.
///
/// Four files said "17 chords" while `KNOWN_DIVERGENCES` held 22
/// ([#440](https://github.com/outlmd/outl/issues/440)); a hand-written
/// count next to a list drifts from it. This pins the other direction
/// too, so the numbers in this file's own doc comment cannot rot.
#[test]
fn the_tui_only_count_matches_the_doc_comment() {
    assert_eq!(
        TUI_ONLY.len(),
        31,
        "TUI_ONLY changed size — update the count in its doc comment and in \
         docs/shortcuts.md, or this number is the next stale one"
    );
}
