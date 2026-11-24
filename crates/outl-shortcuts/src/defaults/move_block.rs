//! Rows that move a block (Normal) or a whole range (Visual) among
//! its siblings. Same owner as `defaults.rs`, split out because that
//! file sits at its size ceiling.
//!
//! Order per action and mode is the platform chord first (`Cmd+Shift`,
//! which `paletteActions` labels from), then `Ctrl+Shift`, `Alt`, and
//! the vim letter last. Each is spelled per OS because the desktop
//! adapter never rewrites `Cmd` to `Ctrl`.
//!
//! The letters are `Shift+k` / `Shift+j`: [`shift_ch`] lowercases, so
//! the chord is a lowercase key with `SHIFT`, which is what the
//! desktop adapter reports for `K`. They never go in `Insert`: a
//! letter there would eat typing.
//!
//! `Insert` carries the three non-letter chords as `MoveBlock*`. The
//! desktop dispatcher treats them as editor-owned (the textarea's own
//! handler runs the move), so the row exists for the TUI parity pins and
//! for `insertMoveFor`, never for a window-level `preventDefault`.

use super::chords::{alt_key, shift_ch, shift_ctrl_key, shift_meta_key};
use crate::action::Action;
use crate::binding::{Binding, Mode};
use crate::chord::{ChordSequence, Key};

pub(super) fn bindings() -> Vec<Binding> {
    let mut rows = Vec::with_capacity(22);
    for (mode, noun, up, down) in [
        (
            Mode::Normal,
            "block",
            Action::MoveBlockUp,
            Action::MoveBlockDown,
        ),
        (
            Mode::Visual,
            "range",
            Action::MoveVisualRangeUp,
            Action::MoveVisualRangeDown,
        ),
    ] {
        for (action, dir, arrow, key, letter) in [
            (up, "up", "Up", Key::Up, 'k'),
            (down, "down", "Down", Key::Down, 'j'),
        ] {
            let chords: [(ChordSequence, String); 4] = [
                (shift_meta_key(key), format!("Cmd+Shift+{arrow}")),
                (shift_ctrl_key(key), format!("Ctrl+Shift+{arrow}")),
                (alt_key(key), format!("Alt+{arrow}")),
                (
                    shift_ch(letter),
                    format!("Shift+{}", letter.to_ascii_uppercase()),
                ),
            ];
            for (chord, spelled) in chords {
                let description = format!("Move {noun} {dir} ({spelled})");
                rows.push(Binding::new(chord, mode, action, description));
            }
        }
    }
    for (action, dir, arrow, key) in [
        (Action::MoveBlockUp, "up", "Up", Key::Up),
        (Action::MoveBlockDown, "down", "Down", Key::Down),
    ] {
        let chords: [(ChordSequence, String); 3] = [
            (shift_meta_key(key), format!("Cmd+Shift+{arrow}")),
            (shift_ctrl_key(key), format!("Ctrl+Shift+{arrow}")),
            (alt_key(key), format!("Alt+{arrow}")),
        ];
        for (chord, spelled) in chords {
            let description = format!("Move block {dir} ({spelled})");
            rows.push(Binding::new(chord, Mode::Insert, action, description));
        }
    }
    rows
}
