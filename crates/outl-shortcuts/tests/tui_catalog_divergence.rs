//! Pinned divergences between this catalog and the TUI's own
//! Normal-mode `match`.
//!
//! # Why this file exists
//!
//! `outl-shortcuts` is the single owner of `(chord → action)`, and
//! [`outl_shortcuts::lookup`] is how a client is *supposed* to reach
//! it. **The TUI does not call it.** `outl-tui/src/input/normal.rs`
//! dispatches Normal-mode keys from a hand-written
//! `match key.code { … }`, and `outl-tui/src/input/chord_adapter.rs`
//! exists to match *plugin* chords and nothing else. So the two
//! halves can disagree about what a key does without anything failing
//! to compile — and they do, on 17 chords, 6 of which mutate the
//! user's content, the op log, or end the session.
//!
//! Root `CLAUDE.md` invariant 12 says a capability difference with no
//! owner is a gap discovered by a user pressing a key. A *chord*
//! difference with no owner is worse: the key is not dead, it does
//! something else. `Ctrl+X` is `CutBlock` in the catalog and deletes
//! a character in the TUI; `Ctrl+Shift+R` is `OpenReminders` in the
//! catalog and redoes in the TUI.
//!
//! # What this file is not
//!
//! It is **not a decision**. Which side is right is a product call —
//! re-spell the catalog, or bind the chord in the TUI — and it
//! belongs to the maintainer, not to the person who noticed. Until
//! then the honest state is the one invariant 12 asks for: the
//! divergence is written down, in code, where it fails if it moves.
//!
//! # What it catches, and what it does not
//!
//! - A catalog row that is **added, removed or re-spelled** →
//!   `every_terminal_reachable_binding_has_a_verdict` (a new chord is
//!   in neither table) and `known_divergences_still_read_the_same`
//!   (a listed chord no longer resolves to the listed action).
//! - A **TUI arm** that is added, removed or re-keyed →
//!   `the_tui_normal_key_arms_have_not_moved`, which pins the arm
//!   patterns of both `match`es in `handle_normal_key` verbatim.
//! - It does **not** prove an arm runs, and it does not infer
//!   meaning from the TUI source. The `tui_does` prose below is
//!   read by a human and pinned only by the arm snapshot underneath
//!   it. That is the same limitation `reminder_chord_tests` in
//!   `outl-tui` declares about itself, for the same reason: nothing
//!   short of running the TUI can close it.
//!
//! # Scope
//!
//! `Mode::Normal` and `Mode::Global` only, and only chords a terminal
//! can actually deliver — every `META` (`Cmd`) spelling is dropped,
//! because crossterm never emits it. Insert, Visual and Overlay have
//! their own TUI handlers and are not audited here.

use outl_shortcuts::{default_bindings, lookup, Action, ChordSequence, Mode, Modifiers};

/// How much a divergence costs the user when they press the key
/// expecting what the catalog (and `docs/shortcuts.md`, and the help
/// overlay) told them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Harm {
    /// Mutates content, mutates the op log, or ends the session.
    Destructive,
    /// Does something else, recoverably.
    Different,
    /// Does nothing. The chord is advertised and dead.
    Dead,
}

/// One chord the catalog and the TUI disagree about.
struct Divergence {
    /// Parsed with [`ChordSequence::parse`] — same spelling the
    /// catalog uses, so a re-spelling in `defaults.rs` fails here.
    chord: &'static str,
    mode: Mode,
    /// What `lookup(mode, chord)` answers today.
    catalog: Action,
    /// What `outl-tui` actually does with the same keystroke.
    tui_does: &'static str,
    harm: Harm,
    /// Why it ended up this way, or what the TUI offers instead.
    note: &'static str,
}

/// The divergences as of this commit. **Decision pending — do not
/// "fix" a row by editing the other side without the maintainer.**
///
/// Removing a row is the right move only when the underlying
/// divergence is gone, in which case
/// `every_terminal_reachable_binding_has_a_verdict` will ask you to
/// move the chord into `AGREED` instead.
const KNOWN_DIVERGENCES: &[Divergence] = &[
    Divergence {
        chord: "Ctrl+}",
        mode: Mode::Global,
        catalog: Action::ZoomIn,
        tui_does: "nothing — there is no arm for `}` at all",
        harm: Harm::Dead,
        note: "The TUI zooms with `z i` / `z o`; the desktop's bracket spelling was \
              never ported.",
    },
    Divergence {
        chord: "Ctrl+{",
        mode: Mode::Global,
        catalog: Action::ZoomOut,
        tui_does: "nothing — there is no arm for `{` at all",
        harm: Harm::Dead,
        note: "Same as `Ctrl+}` above.",
    },
    Divergence {
        chord: "Home",
        mode: Mode::Normal,
        catalog: Action::OpenToday,
        tui_does: "moves the cursor to the start of the block (`cursor_to_home`)",
        harm: Harm::Different,
        note: "`input/normal.rs` says in a comment that `t` and `Home` were \
              deliberately split; the catalog kept the pre-split spelling.",
    },
    Divergence {
        chord: "Ctrl+Shift+r",
        mode: Mode::Global,
        catalog: Action::OpenReminders,
        tui_does: "redo — an op-log mutation — on a terminal without the kitty protocol, \
              where `Ctrl+Shift+R` collapses to `Ctrl+R`; nothing on a terminal with \
              it, where the arm's lowercase `'r'` never matches `Char('R')`",
        harm: Harm::Destructive,
        note: "`defaults.rs` says the TUI took `g n` *because* `Ctrl+R` is Redo, then \
              bound the desktop spelling `Global`, which is every client.",
    },
    Divergence {
        chord: "Ctrl+Shift+p",
        mode: Mode::Normal,
        catalog: Action::AddProperty,
        tui_does: "opens the quick switcher on a terminal without the kitty protocol \
              (collapses to `Ctrl+P`); nothing on a terminal with it",
        harm: Harm::Different,
        note: "Same collapse as `Ctrl+Shift+R`. The TUI reaches the property editor \
              with `g p`.",
    },
    Divergence {
        chord: "Ctrl+Shift+Enter",
        mode: Mode::Normal,
        catalog: Action::NewBlockBelow,
        tui_does: "toggles TODO / DONE on the selected block — the `Enter if CONTROL` arm \
              does not exclude SHIFT",
        harm: Harm::Destructive,
        note: "Writes a marker into the block text through the op log.",
    },
    Divergence {
        chord: "Ctrl+Shift+Up",
        mode: Mode::Normal,
        catalog: Action::MoveBlockUp,
        tui_does: "moves the *selection* up one block, not the block — `Up | Char('k')` is \
              unguarded and catches the modifier",
        harm: Harm::Different,
        note: "The TUI reorders with `K` / `Alt+Up`.",
    },
    Divergence {
        chord: "Ctrl+Shift+Down",
        mode: Mode::Normal,
        catalog: Action::MoveBlockDown,
        tui_does: "moves the *selection* down one block, not the block",
        harm: Harm::Different,
        note: "The TUI reorders with `J` / `Alt+Down`.",
    },
    Divergence {
        chord: "Ctrl+x",
        mode: Mode::Normal,
        catalog: Action::CutBlock,
        tui_does: "deletes the character under the cursor — `KeyCode::Char('x')` is \
              unguarded, so it catches `Ctrl+X` too",
        harm: Harm::Destructive,
        note: "The catalog's own comment calls `Cmd/Ctrl+X` the OS-native cut. In the \
              TUI it silently eats a character.",
    },
    Divergence {
        chord: "Ctrl+c",
        mode: Mode::Normal,
        catalog: Action::CopyBlock,
        tui_does: "quits the TUI — `runtime.rs` intercepts `Ctrl+C` before \
              `handle_normal_key` ever runs",
        harm: Harm::Destructive,
        note: "The catalog binds this chord twice: `Global` → Quit and `Normal` → \
              CopyBlock. `lookup(Normal, Ctrl+C)` answers CopyBlock, so the two rows \
              already disagree with each other before the TUI is consulted.",
    },
    Divergence {
        chord: "Ctrl+v",
        mode: Mode::Normal,
        catalog: Action::PasteBlock,
        tui_does: "nothing — there is no `Char('v')` arm, only `Char('V')` for Visual",
        harm: Harm::Dead,
        note: "The TUI pastes with `p` / `P` (OS clipboard), which the catalog does not \
              bind at all.",
    },
    Divergence {
        chord: "Esc",
        mode: Mode::Normal,
        catalog: Action::ExitInsert,
        tui_does: "nothing — Normal mode has no `Esc` arm outside the sidebar and \
              pending-op intercepts",
        harm: Harm::Dead,
        note: "The catalog's row is \"cancel pending cut\", and the TUI has no \
              pending-cut state to cancel.",
    },
    Divergence {
        chord: "v",
        mode: Mode::Normal,
        catalog: Action::EnterVisual,
        tui_does: "nothing — the TUI enters Visual with `V` only",
        harm: Harm::Dead,
        note: "Adding a lowercase arm would close this; it is a one-line gap, not a \
              design difference.",
    },
    Divergence {
        chord: "Shift+Down",
        mode: Mode::Normal,
        catalog: Action::SelectRangeDown,
        tui_does: "moves the selection down one block — no range is started",
        harm: Harm::Different,
        note: "The TUI has Visual (`V`) and no non-vim range-select entry.",
    },
    Divergence {
        chord: "Shift+Up",
        mode: Mode::Normal,
        catalog: Action::SelectRangeUp,
        tui_does: "moves the selection up one block — no range is started",
        harm: Harm::Different,
        note: "Mirror of `Shift+Down`.",
    },
    Divergence {
        chord: "Ctrl+z",
        mode: Mode::Normal,
        catalog: Action::Undo,
        tui_does: "arms the `z` fold-chord family; the *next* key resolves in the bare-key \
              arms, where a lowercase `r` arms replace-char and the key after that \
              overwrites a character",
        harm: Harm::Destructive,
        note: "Two keystrokes from `Ctrl+Z` to a silent content edit. The TUI undoes \
              with `u`.",
    },
    Divergence {
        chord: "Ctrl+Shift+z",
        mode: Mode::Normal,
        catalog: Action::Redo,
        tui_does: "the same `z`-chord arming as `Ctrl+Z` on a terminal without the kitty \
              protocol; nothing on a terminal with it",
        harm: Harm::Destructive,
        note: "Same path as `Ctrl+Z`. The TUI redoes with `Ctrl+R`.",
    },
];

/// Every other terminal-reachable `Normal` / `Global` binding: the
/// catalog and the TUI agree on what the chord does.
///
/// This table exists for coverage, not for its own sake — it is what
/// lets `every_terminal_reachable_binding_has_a_verdict` tell "a
/// chord somebody checked" apart from "a chord nobody has looked at
/// since it was added".
const AGREED: &[(&str, Mode, Action)] = &[
    ("Ctrl+p", Mode::Global, Action::OpenPicker),
    ("Ctrl+c", Mode::Global, Action::Quit),
    ("Ctrl+Enter", Mode::Normal, Action::ToggleTodo),
    ("Ctrl+t", Mode::Normal, Action::ToggleTodo),
    ("g x", Mode::Normal, Action::RunCodeBlock),
    ("t", Mode::Normal, Action::OpenToday),
    ("[", Mode::Normal, Action::PrevDay),
    ("]", Mode::Normal, Action::NextDay),
    ("g j", Mode::Normal, Action::OpenToday),
    ("g d", Mode::Normal, Action::DeletePage),
    ("g r", Mode::Normal, Action::InsertRemind),
    ("g Shift+r", Mode::Normal, Action::InsertRemindNag),
    ("g n", Mode::Normal, Action::OpenReminders),
    ("g s", Mode::Normal, Action::SnoozeReminder),
    ("g p", Mode::Normal, Action::OpenProperties),
    ("g Shift+p", Mode::Normal, Action::TogglePin),
    ("Ctrl+p", Mode::Normal, Action::OpenPicker),
    ("?", Mode::Normal, Action::ToggleHelp),
    (":", Mode::Normal, Action::OpenCommandPalette),
    ("q q", Mode::Normal, Action::Quit),
    ("Shift+z Shift+z", Mode::Normal, Action::Quit),
    ("j", Mode::Normal, Action::SelectionDown),
    ("Down", Mode::Normal, Action::SelectionDown),
    ("k", Mode::Normal, Action::SelectionUp),
    ("Up", Mode::Normal, Action::SelectionUp),
    ("i", Mode::Normal, Action::EnterInsert),
    ("Shift+i", Mode::Normal, Action::EnterInsertAtStart),
    ("a", Mode::Normal, Action::EnterInsertAfter),
    ("Shift+a", Mode::Normal, Action::EnterInsertAtEnd),
    ("x", Mode::Normal, Action::DeleteCharUnderCursor),
    ("Shift+x", Mode::Normal, Action::DeleteCharBeforeCursor),
    ("Shift+d", Mode::Normal, Action::DeleteToEndOfBlock),
    ("Shift+c", Mode::Normal, Action::ChangeToEndOfBlock),
    ("Shift+s", Mode::Normal, Action::SubstituteBlock),
    ("s", Mode::Normal, Action::SubstituteChar),
    ("r", Mode::Normal, Action::ReplaceChar),
    ("f", Mode::Normal, Action::FindCharForward),
    ("Shift+f", Mode::Normal, Action::FindCharBackward),
    ("~", Mode::Normal, Action::ToggleCharCase),
    ("Shift+y", Mode::Normal, Action::YankCurrentBlock),
    ("e", Mode::Normal, Action::CursorWordEnd),
    ("*", Mode::Normal, Action::SearchWordForward),
    ("#", Mode::Normal, Action::SearchWordBackward),
    ("z Shift+r", Mode::Normal, Action::UnfoldAll),
    ("z Shift+m", Mode::Normal, Action::FoldAll),
    ("z z", Mode::Normal, Action::CenterViewport),
    ("z i", Mode::Normal, Action::ZoomIn),
    ("z o", Mode::Normal, Action::ZoomOut),
    ("g v", Mode::Normal, Action::ReselectLastVisual),
    ("Enter", Mode::Normal, Action::OpenRefUnderCursor),
    ("o", Mode::Normal, Action::NewBlockBelow),
    ("Shift+o", Mode::Normal, Action::NewBlockAbove),
    ("Tab", Mode::Normal, Action::IndentBlock),
    ("Shift+Tab", Mode::Normal, Action::OutdentBlock),
    ("d d", Mode::Normal, Action::DeleteBlock),
    ("c", Mode::Normal, Action::ToggleCollapsed),
    ("y r", Mode::Normal, Action::CopyBlockRef),
    ("u", Mode::Normal, Action::Undo),
    ("Ctrl+r", Mode::Normal, Action::Redo),
];

fn seq(s: &str) -> ChordSequence {
    ChordSequence::parse(s).unwrap_or_else(|| panic!("unparseable chord in this test: {s:?}"))
}

/// Bindings a terminal can deliver: `Normal` / `Global`, no `META`.
fn terminal_reachable() -> Vec<(Mode, ChordSequence, Action)> {
    default_bindings()
        .into_iter()
        .filter(|b| matches!(b.mode, Mode::Normal | Mode::Global))
        .filter(|b| !b.chord.0.iter().any(|c| c.mods.contains(Modifiers::META)))
        .map(|b| (b.mode, b.chord, b.action))
        .collect()
}

#[test]
fn known_divergences_still_read_the_same() {
    // Pins the catalog half of every row. If someone re-spells a
    // chord in `defaults.rs` (the `pair('z', 'R')` → `z r`
    // lowercasing bug was exactly that) or points it at a different
    // action, the row stops describing reality and this fails
    // instead of the list quietly going stale.
    for d in KNOWN_DIVERGENCES {
        assert_eq!(
            lookup(d.mode, &seq(d.chord)),
            Some(d.catalog),
            "{} in {:?} no longer resolves to {:?} — the catalog moved, so this \
             divergence row now describes something that does not exist. Re-audit \
             the chord against outl-tui/src/input/normal.rs before editing the row.",
            d.chord,
            d.mode,
            d.catalog,
        );
    }
}

#[test]
fn the_divergence_list_has_not_silently_shrunk() {
    // A row deleted without the divergence being resolved is the
    // failure mode this whole file exists to prevent, and deleting
    // one is a single keystroke. Pin the shape of the list.
    assert_eq!(
        KNOWN_DIVERGENCES.len(),
        17,
        "the known-divergence list changed size — if a divergence was genuinely \
         resolved, move the chord to AGREED and update this count in the same edit",
    );
    let destructive = KNOWN_DIVERGENCES
        .iter()
        .filter(|d| d.harm == Harm::Destructive)
        .count();
    assert_eq!(
        destructive, 6,
        "the count of destructive divergences changed — these are the rows where \
         the user loses content, an op, or the session, so a change here is the \
         one that needs saying out loud",
    );
    for d in KNOWN_DIVERGENCES {
        assert!(
            !d.tui_does.trim().is_empty() && !d.note.trim().is_empty(),
            "{} has an empty explanation — a row nobody can read is a row nobody acts on",
            d.chord,
        );
    }
}

#[test]
fn no_chord_is_both_agreed_and_divergent() {
    for d in KNOWN_DIVERGENCES {
        let s = seq(d.chord);
        assert!(
            !AGREED.iter().any(|(c, m, _)| *m == d.mode && seq(c) == s),
            "{} in {:?} is in both tables",
            d.chord,
            d.mode,
        );
    }
}

#[test]
fn agreed_rows_still_resolve_to_what_they_claim() {
    for (chord, mode, action) in AGREED {
        assert_eq!(
            lookup(*mode, &seq(chord)),
            Some(*action),
            "{chord} in {mode:?} no longer resolves to {action:?}",
        );
    }
}

#[test]
fn every_terminal_reachable_binding_has_a_verdict() {
    // The coverage gate. A chord added to `defaults.rs` in `Normal`
    // or `Global` without a `META` modifier is a chord the TUI will
    // either honour or quietly do something else with, and there is
    // no compiler to ask. This is what makes the author look.
    let mut unaccounted = Vec::new();
    for (mode, chord, action) in terminal_reachable() {
        let agreed = AGREED.iter().any(|(c, m, _)| *m == mode && seq(c) == chord);
        let known = KNOWN_DIVERGENCES
            .iter()
            .any(|d| d.mode == mode && seq(d.chord) == chord);
        if !agreed && !known {
            unaccounted.push(format!("{mode:?} {chord:?} → {action:?}"));
        }
    }
    assert!(
        unaccounted.is_empty(),
        "these Normal/Global chords have no verdict against the TUI's own match in \
         outl-tui/src/input/normal.rs:\n  {}\n\nCheck what the TUI does with each, \
         then add it to AGREED or to KNOWN_DIVERGENCES. Do not guess: the TUI's \
         bare-key arms are mostly unguarded, so a Ctrl+ chord usually lands on the \
         plain-letter arm.",
        unaccounted.join("\n  "),
    );
}

/// The TUI half: arm-pattern snapshots scraped from
/// `outl-tui/src/input/normal.rs` and `runtime.rs`, in their own
/// module so this file stays readable. Its `#[test]`s run as part of
/// this target.
mod tui_normal_arms;
