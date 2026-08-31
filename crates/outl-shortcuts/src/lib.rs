//! # outl-shortcuts
//!
//! Single source of truth for outl's keyboard bindings, consumed by
//! every client that paints an editor:
//!
//! - **`outl-tui`** translates `crossterm::event::KeyEvent` to
//!   [`Chord`] and looks up the binding for its current [`Mode`].
//! - **`outl-desktop`** exposes [`default_bindings`] over a Tauri
//!   command; the Solid frontend translates `KeyboardEvent` to
//!   [`Chord`], looks up the binding, and dispatches to the JS
//!   handler that maps each [`Action`] to a Tauri call.
//!
//! Both clients see the same `(chord → action)` mapping, so a key
//! the user knows from the TUI works identically in the desktop.
//! A user-level override (TOML / settings) plugs into the same
//! pipeline — same `Binding`, different source list.
//!
//! ## What this crate owns
//!
//! - The [`Action`] enum — every named operation outl performs in
//!   response to a key. Adding an entry here is a coordinated
//!   commit: clients have to add a handler for it, but the *name*
//!   stays one place.
//! - The [`Chord`] / [`ChordSequence`] types — modifiers + key
//!   prefix, expressed independently of any input library so both
//!   crossterm (TUI) and the browser DOM (desktop) can map into
//!   them.
//! - The [`Mode`] enum — which modal state a binding applies to.
//!   `Global` matches everywhere; `Normal` / `Insert` / `Visual` /
//!   `Overlay` are the TUI's vim modes (desktop subscribes only
//!   while `settings.vim_mode == true`).
//! - The default binding table ([`default_bindings`]) and helpers
//!   to query it by mode or action.
//!
//! ## What this crate does NOT own
//!
//! - Handlers. Each client maps `Action -> {do_something}` itself.
//!   This crate doesn't know how the TUI commits an insert buffer
//!   or how the desktop calls `paste_markdown_at`.
//! - Input adapters. `crossterm::KeyEvent -> Chord` lives in
//!   `outl-tui`; `KeyboardEvent -> Chord` lives in the desktop's
//!   `lib/shortcuts.ts`. Both produce a [`Chord`] this crate then
//!   resolves.

mod action;
mod binding;
mod chord;
mod defaults;
mod support;

pub use action::Action;
pub use binding::{Binding, Mode};
pub use chord::{Chord, ChordSequence, Key, Modifiers};
pub use defaults::default_bindings;
pub use support::{support, Client, ClientSupport, Support};

/// Return every default binding active in `mode`, in the order
/// they appear in [`default_bindings`]. Bindings tagged
/// [`Mode::Global`] are included unconditionally — chrome shortcuts
/// like `Cmd+P` work everywhere.
pub fn bindings_for_mode(mode: Mode) -> Vec<Binding> {
    default_bindings()
        .into_iter()
        .filter(|b| b.mode == Mode::Global || b.mode == mode)
        .collect()
}

/// Look up the action triggered by `seq` when the editor is in
/// `mode`. Returns `None` when nothing matches — the client falls
/// back to whatever its default key handling does (typing into a
/// textarea, …).
///
/// **Mode-specific bindings take precedence over `Global` ones.** A
/// chord can be bound twice — once for a concrete mode, once for
/// `Global` — and the concrete mode must win when the editor is in
/// it. `Cmd+Shift+X` is the canonical case: `WrapStrike` in `Insert`
/// (textarea focused), `RunCodeBlock` everywhere else via `Global`.
/// We can't lean on table order here because the `Global` chrome rows
/// are listed first for help-overlay readability, so a plain
/// first-match `find` would resolve every dual-bound chord to its
/// `Global` action even inside the mode that should override it.
///
/// Prefix matching is the caller's job: when a [`ChordSequence`]
/// has two chords (e.g. `g j`), the caller buffers the first chord
/// and re-queries with the full sequence on the second key.
pub fn lookup(mode: Mode, seq: &ChordSequence) -> Option<Action> {
    let mut global_match = None;
    for b in default_bindings() {
        if &b.chord != seq {
            continue;
        }
        if b.mode == mode {
            // Exact-mode match wins outright, regardless of where it
            // sits in the table relative to the Global row.
            return Some(b.action);
        }
        if b.mode == Mode::Global {
            global_match = Some(b.action);
        }
    }
    global_match
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_has_a_description() {
        for b in default_bindings() {
            assert!(
                !b.description.is_empty(),
                "binding for {:?} ({:?}) has empty description",
                b.action,
                b.chord
            );
        }
    }

    #[test]
    fn no_duplicate_chord_in_same_mode() {
        // Two bindings sharing the same `(mode, chord)` would make
        // `lookup` non-deterministic — guard against it at the
        // catalog level so a typo is caught by `cargo test`.
        let bindings = default_bindings();
        for (i, a) in bindings.iter().enumerate() {
            for b in &bindings[i + 1..] {
                if a.mode == b.mode && a.chord == b.chord {
                    panic!(
                        "duplicate binding in mode {:?}: {:?} maps to both {:?} and {:?}",
                        a.mode, a.chord, a.action, b.action
                    );
                }
            }
        }
    }

    #[test]
    fn bindings_round_trip_via_serde() {
        let bs = default_bindings();
        let json = serde_json::to_string(&bs).unwrap();
        let back: Vec<Binding> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), bs.len());
    }

    #[test]
    fn global_chrome_shortcuts_resolve_in_every_mode() {
        // `Cmd+P` is the canonical chrome shortcut — must work in
        // Normal, Insert, Visual, Overlay. We test against any
        // chord wired to `OpenPicker` in `Global` mode.
        let modes = [Mode::Normal, Mode::Insert, Mode::Visual, Mode::Overlay];
        let picker_bindings: Vec<_> = default_bindings()
            .into_iter()
            .filter(|b| b.action == Action::OpenPicker && b.mode == Mode::Global)
            .collect();
        assert!(
            !picker_bindings.is_empty(),
            "no Global binding for OpenPicker"
        );
        let chord = &picker_bindings[0].chord;
        for mode in modes {
            assert_eq!(
                lookup(mode, chord),
                Some(Action::OpenPicker),
                "OpenPicker not reachable in {mode:?}",
            );
        }
    }

    #[test]
    fn cmd_shift_x_splits_between_insert_and_global() {
        // `Cmd+Shift+X` is the canonical "same chord, two actions
        // across modes" case (see `defaults.rs` + the crate
        // CLAUDE.md). Inside a textarea (Insert) the mode-specific
        // `WrapStrike` row must win over the Global `RunCodeBlock`
        // one; everywhere else the Global binding fires. If this
        // split ever regresses, running a fenced block while editing
        // it would clobber the user's selection with strikethrough.
        let shift_meta_x = ChordSequence::chord(Chord::new(
            Modifiers::META | Modifiers::SHIFT,
            Key::char('x'),
        ));

        assert_eq!(
            lookup(Mode::Insert, &shift_meta_x),
            Some(Action::WrapStrike),
            "Cmd+Shift+X in Insert must resolve to WrapStrike (mode-specific beats Global)",
        );

        for mode in [Mode::Normal, Mode::Visual, Mode::Overlay] {
            assert_eq!(
                lookup(mode, &shift_meta_x),
                Some(Action::RunCodeBlock),
                "Cmd+Shift+X must fall through to the Global RunCodeBlock in {mode:?}",
            );
        }
    }

    #[test]
    fn cmd_shift_enter_splits_between_insert_and_normal() {
        // `Cmd+Shift+Enter` is dual-bound (issue #92): in Insert it
        // commits the in-flight edit and starts the next block
        // (`CommitAndContinue`); in view mode (Normal — the desktop's
        // fallback whenever no textarea is focused) there's nothing to
        // commit, so it creates a sibling below via `NewBlockBelow`.
        // The two rows live in different modes, so `lookup`'s
        // mode-specific resolution must keep them apart. No Global row
        // backs this chord, so the remaining modes resolve to nothing.
        //
        // The chord is also dual-spelled per OS: `Cmd+Shift+Enter`
        // (META) on macOS and `Ctrl+Shift+Enter` (CTRL) on Windows /
        // Linux. Both spellings must resolve identically in each mode —
        // the desktop adapter never rewrites `Cmd`↔`Ctrl`, so a missing
        // CTRL row would silently drop the chord off Windows / Linux.
        let shift_meta_enter =
            ChordSequence::chord(Chord::new(Modifiers::META | Modifiers::SHIFT, Key::Enter));
        let shift_ctrl_enter =
            ChordSequence::chord(Chord::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Enter));

        for chord in [&shift_meta_enter, &shift_ctrl_enter] {
            assert_eq!(
                lookup(Mode::Insert, chord),
                Some(Action::CommitAndContinue),
                "Shift+Enter chord in Insert must commit + start the next block",
            );
            assert_eq!(
                lookup(Mode::Normal, chord),
                Some(Action::NewBlockBelow),
                "Shift+Enter chord in Normal (view mode) must create a block below",
            );
            for mode in [Mode::Visual, Mode::Overlay, Mode::Global] {
                assert_eq!(
                    lookup(mode, chord),
                    None,
                    "Shift+Enter chord has no binding in {mode:?}",
                );
            }
        }
    }

    #[test]
    fn zoom_chords_match_the_desktop_adapter_output() {
        // Regression (PR #177 review): the desktop `KeyboardEvent`
        // adapter (`outl-desktop/src/lib/shortcuts.ts::modsOf`) drops
        // the SHIFT bit for symbol keys and reports the already-shifted
        // glyph, so pressing `Cmd/Ctrl+Shift+]` reaches `lookup` as
        // `META/CTRL + Char('}')` — NOT `META/CTRL | SHIFT + Char(']')`.
        // The zoom bindings were originally spelled `shift_meta_ch(']')`
        // and never fired on the desktop. Assert against the exact chord
        // the adapter produces so the split can't silently regress.
        let zoom_in_meta = ChordSequence::chord(Chord::new(Modifiers::META, Key::char('}')));
        let zoom_in_ctrl = ChordSequence::chord(Chord::new(Modifiers::CTRL, Key::char('}')));
        let zoom_out_meta = ChordSequence::chord(Chord::new(Modifiers::META, Key::char('{')));
        let zoom_out_ctrl = ChordSequence::chord(Chord::new(Modifiers::CTRL, Key::char('{')));

        for chord in [&zoom_in_meta, &zoom_in_ctrl] {
            assert_eq!(
                lookup(Mode::Global, chord),
                Some(Action::ZoomIn),
                "Cmd/Ctrl+Shift+] (adapter emits {chord:?}) must resolve to ZoomIn",
            );
        }
        for chord in [&zoom_out_meta, &zoom_out_ctrl] {
            assert_eq!(
                lookup(Mode::Global, chord),
                Some(Action::ZoomOut),
                "Cmd/Ctrl+Shift+[ (adapter emits {chord:?}) must resolve to ZoomOut",
            );
        }
    }
}
