//! Chord constructors for the binding table.
//!
//! One-liners, named so a row in `defaults.rs` reads as the chord a
//! user would say out loud. Split out of that file only because it
//! is at its size ceiling; this is the same owner, not a new one.
//!
//! **`ch` lowercases** — [`Key::char`] runs `to_ascii_lowercase`, and
//! so therefore does [`pair`]. A shifted *second* key needs
//! [`pair_shift_second`]: writing `pair('z', 'R')` silently spells
//! `z r`, which is how `zR` / `zM` sat mis-spelt in the catalog while
//! `z` then a lowercase `r` fell through to the TUI's replace-char
//! arm. The compiler cannot see the difference; only a test can.

use crate::chord::{Chord, ChordSequence, Key, Modifiers};

pub(super) fn ctrl(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::ctrl(c))
}
pub(super) fn meta(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::meta(c))
}
pub(super) fn ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::ch(c))
}
pub(super) fn key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::plain(k))
}
pub(super) fn shift(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::SHIFT, k))
}
pub(super) fn shift_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::SHIFT, Key::char(c)))
}
pub(super) fn shift_meta_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META | Modifiers::SHIFT, Key::char(c)))
}
pub(super) fn shift_ctrl_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::CTRL | Modifiers::SHIFT, Key::char(c)))
}
pub(super) fn meta_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META, k))
}
pub(super) fn shift_meta_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META | Modifiers::SHIFT, k))
}
pub(super) fn ctrl_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::CTRL, k))
}
pub(super) fn shift_ctrl_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::CTRL | Modifiers::SHIFT, k))
}
pub(super) fn pair(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(Chord::ch(a), Chord::ch(b))
}
/// `g` then `Shift+r` — a lead-in char followed by a shifted one.
/// Distinct from [`shift_pair`], which shifts **both** (that's `ZZ`).
pub(super) fn pair_shift_second(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(Chord::ch(a), Chord::new(Modifiers::SHIFT, Key::char(b)))
}
pub(super) fn shift_pair(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(
        Chord::new(Modifiers::SHIFT, Key::char(a)),
        Chord::new(Modifiers::SHIFT, Key::char(b)),
    )
}
