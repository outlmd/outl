//! Drift guard between the shared chord catalog and this file.
//!
//! The TUI dispatches Normal-mode keys through its own `match` here
//! rather than `outl_shortcuts::lookup`, so the catalog and the
//! match arms can disagree without anything failing to compile.
//! These tests pin the reminder chords on the catalog side; if
//! someone re-spells one there, this fails and points at the arm
//! that needs the same edit.
//!
//! They do not prove the arm *runs* (that needs a live `App` with a
//! workspace). They prove the two halves still agree on the
//! spelling, which is the half that silently rots.

use outl_shortcuts::{lookup, Action, Chord, ChordSequence, Key, Mode, Modifiers};

fn pair(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(Chord::ch(a), Chord::ch(b))
}

#[test]
fn g_r_still_means_insert_remind() {
    assert_eq!(
        lookup(Mode::Normal, &pair('g', 'r')),
        Some(Action::InsertRemind),
        "the `('g', Char('r'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn g_shift_r_still_means_the_nag_preset() {
    let seq = ChordSequence::pair(Chord::ch('g'), Chord::new(Modifiers::SHIFT, Key::char('r')));
    assert_eq!(
        lookup(Mode::Normal, &seq),
        Some(Action::InsertRemindNag),
        "the `('g', Char('R'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn g_n_still_opens_the_reminders_overlay() {
    assert_eq!(
        lookup(Mode::Normal, &pair('g', 'n')),
        Some(Action::OpenReminders),
        "the `('g', Char('n'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn g_s_still_snoozes_the_selected_block() {
    assert_eq!(
        lookup(Mode::Normal, &pair('g', 's')),
        Some(Action::SnoozeReminder),
        "the `('g', Char('s'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn g_p_opens_the_property_editor() {
    assert_eq!(
        lookup(Mode::Normal, &pair('g', 'p')),
        Some(Action::OpenProperties),
        "the `('g', Char('p'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn g_shift_p_is_the_pin_toggle_that_g_p_displaced() {
    // `g p` meant pin until the property editor took the
    // spelling. If someone reverts one half of that move, the
    // other half silently becomes unreachable — this is the test
    // that notices.
    let seq = ChordSequence::pair(Chord::ch('g'), Chord::new(Modifiers::SHIFT, Key::char('p')));
    assert_eq!(
        lookup(Mode::Normal, &seq),
        Some(Action::TogglePin),
        "the `('g', Char('P'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn g_shift_d_still_means_mark_done() {
    // `g D` lands a block on DONE outright, distinct from `Ctrl+T`'s
    // TODO → DOING → DONE cycle. Shifted so it can't be fat-fingered
    // instead of `g d` (delete the page) — a near-miss between "finish
    // this task" and "delete this whole page".
    let seq = ChordSequence::pair(Chord::ch('g'), Chord::new(Modifiers::SHIFT, Key::char('d')));
    assert_eq!(
        lookup(Mode::Normal, &seq),
        Some(Action::MarkDone),
        "the `('g', Char('D'))` arm in handle_normal_key must match this"
    );
}

#[test]
fn ctrl_p_is_still_the_quick_switcher_not_properties() {
    // `g p` is a two-key chord; a bare `Ctrl+P` must keep opening
    // the picker, which is the muscle memory most users have.
    assert_eq!(
        lookup(Mode::Normal, &ChordSequence::chord(Chord::ctrl('p'))),
        Some(Action::OpenPicker)
    );
}

#[test]
fn ctrl_r_is_still_redo_not_reminders() {
    // The issue sketched `Ctrl+R` for the reminders overlay. It is
    // Redo, and a terminal can't tell it from `Ctrl+Shift+R`, which
    // is why the TUI took `g n` instead. If this ever flips, the
    // user loses undo/redo to a list nobody asked to open.
    assert_eq!(
        lookup(Mode::Normal, &ChordSequence::chord(Chord::ctrl('r'))),
        Some(Action::Redo)
    );
}

#[test]
fn g_o_opens_the_backlink_context() {
    assert_eq!(
        lookup(Mode::Normal, &pair('g', 'o')),
        Some(Action::OpenBacklinkContext),
        "the `('g', Char('o'))` arm in handle_normal_key must match this"
    );
}
