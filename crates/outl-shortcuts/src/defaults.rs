//! Default binding catalog.
//!
//! Hand-curated table seeded from `outl-tui/src/input/` (the
//! shipping shortcuts) plus the OS-standard chrome bindings the
//! desktop already relied on (`Cmd/Ctrl+P`, `Cmd/Ctrl+B`, etc.).
//!
//! Adding a binding here lights it up on both clients
//! simultaneously — the TUI's chord adapter and the desktop's
//! `KeyboardEvent` adapter both go through the same lookup.

use crate::action::Action;
use crate::binding::{Binding, Mode};
use crate::chord::{Chord, ChordSequence, Key, Modifiers};

fn ctrl(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::ctrl(c))
}
fn meta(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::meta(c))
}
fn ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::ch(c))
}
fn key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::plain(k))
}
fn shift(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::SHIFT, k))
}
fn shift_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::SHIFT, Key::char(c)))
}
fn shift_meta_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META | Modifiers::SHIFT, Key::char(c)))
}
fn shift_ctrl_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::CTRL | Modifiers::SHIFT, Key::char(c)))
}
fn meta_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META, k))
}
fn shift_meta_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META | Modifiers::SHIFT, k))
}
fn ctrl_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::CTRL, k))
}
fn shift_ctrl_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::CTRL | Modifiers::SHIFT, k))
}
fn pair(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(Chord::ch(a), Chord::ch(b))
}
/// `g` then `Shift+r` — a lead-in char followed by a shifted one.
/// Distinct from [`shift_pair`], which shifts **both** (that's `ZZ`).
fn pair_shift_second(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(Chord::ch(a), Chord::new(Modifiers::SHIFT, Key::char(b)))
}
fn shift_pair(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(
        Chord::new(Modifiers::SHIFT, Key::char(a)),
        Chord::new(Modifiers::SHIFT, Key::char(b)),
    )
}

/// Every binding outl ships with by default.
///
/// `Global` entries fire in every mode — those are the OS-standard
/// chrome chords (`Cmd+P`, `Cmd+T`, `Cmd+,`, `Cmd+[`, `Cmd+]`,
/// `Cmd+Shift+E`, `Cmd+Shift+B`). The TUI adapter doesn't emit
/// `META` from crossterm, so on a terminal these manifest as their
/// TUI-native variants (single letters, brackets, …).
///
/// Two chords were deliberately freed up vs. the first cut:
///
/// - `Cmd+B` is **reserved for `WrapBold`** in Insert mode, matching
///   every markdown editor on the planet (Notion, Obsidian, Discord,
///   Slack, Typora). It would be hostile to retrain users on a
///   non-standard meaning.
/// - `Cmd+\` is **1Password's** global autofill shortcut on macOS.
///   Hijacking it breaks every user who has 1Password installed.
///
/// Sidebar / backlinks panel toggles ride `Cmd+Shift+E` (mirrors
/// VS Code's "explorer" pane) and `Cmd+Shift+B`.
pub fn default_bindings() -> Vec<Binding> {
    use Mode::*;

    vec![
        // ── Global chrome (OS-standard, work in every mode) ───────
        Binding::new(meta('p'), Global, Action::OpenPicker, "Open quick switcher"),
        Binding::new(ctrl('p'), Global, Action::OpenPicker, "Open quick switcher"),
        // "Journal" mnemonic — `Cmd+J` opens today's journal. `Cmd+T`
        // used to live here but conflicted with the muscle memory
        // every outliner has: `T` for *task* / TODO. We freed `Cmd+T`
        // for the ToggleTodo binding below.
        Binding::new(meta('j'), Global, Action::OpenToday, "Open today's journal"),
        Binding::new(
            shift_meta_ch('e'),
            Global,
            Action::ToggleSidebar,
            "Toggle sidebar",
        ),
        Binding::new(
            shift_meta_ch('b'),
            Global,
            Action::ToggleBacklinks,
            "Toggle backlinks panel",
        ),
        Binding::new(meta(','), Global, Action::OpenSettings, "Open settings"),
        Binding::new(meta('['), Global, Action::PrevDay, "Previous journal day"),
        Binding::new(meta(']'), Global, Action::NextDay, "Next journal day"),
        // Zoom into / out of a block for the non-vim desktop. Mirrors the
        // `Cmd+[`/`Cmd+]` day-navigation pair one dimension over: plain
        // brackets walk days, `Shift`+brackets walk the block hierarchy.
        // `]` = deeper (zoom in), `[` = back out, matching the browser
        // back/forward convention. The user presses `Cmd/Ctrl+Shift+]`,
        // but the catalog binds the **shifted glyph without SHIFT**
        // (`}` / `{`): the desktop `KeyboardEvent` adapter
        // (`shortcuts.ts::modsOf`) deliberately drops the SHIFT bit for
        // symbol keys and reports the already-shifted `e.key` (`}`/`{`),
        // the same convention `ch('?')`-style bindings use. Binding
        // `SHIFT + ]` here would never match on a US layout. Dual-spelled
        // META + CTRL because the desktop adapter never rewrites
        // `Cmd`↔`Ctrl`.
        Binding::new(
            meta('}'),
            Global,
            Action::ZoomIn,
            "Zoom in on block (Cmd+Shift+])",
        ),
        Binding::new(
            ctrl('}'),
            Global,
            Action::ZoomIn,
            "Zoom in on block (Ctrl+Shift+])",
        ),
        Binding::new(meta('{'), Global, Action::ZoomOut, "Zoom out (Cmd+Shift+[)"),
        Binding::new(
            ctrl('{'),
            Global,
            Action::ZoomOut,
            "Zoom out (Ctrl+Shift+[)",
        ),
        Binding::new(ctrl('c'), Global, Action::Quit, "Quit"),
        // Toggle TODO/DONE.
        // - Desktop: `Cmd+Enter` and `Cmd+T` (Global). Both fire
        //   inside a textarea (Insert) and on a selected block
        //   (Normal). `Cmd+T` mirrors the TUI's `Ctrl+T` and the
        //   "T for task" muscle memory; `Cmd+Enter` is the "commit
        //   the state of this row" gesture.
        // - TUI: `Ctrl+Enter` and the legacy `Ctrl+T` in Normal mode.
        Binding::new(
            meta_key(Key::Enter),
            Global,
            Action::ToggleTodo,
            "Toggle TODO / DONE",
        ),
        Binding::new(
            meta('t'),
            Global,
            Action::ToggleTodo,
            "Toggle TODO / DONE (T for task)",
        ),
        Binding::new(
            ctrl_key(Key::Enter),
            Normal,
            Action::ToggleTodo,
            "Toggle TODO / DONE (TUI)",
        ),
        Binding::new(
            ctrl('t'),
            Normal,
            Action::ToggleTodo,
            "Toggle TODO / DONE (TUI alt)",
        ),
        // Run the fenced code block under the cursor / focused
        // block. Desktop: `Cmd+Shift+X`, bound **Global** so it fires
        // in view mode and in Visual — inside a textarea the
        // Insert-mode `WrapStrike` binding below wins (mode-specific
        // beats Global), so running a block you're editing means
        // committing first or using the per-block run button. See
        // issue #80. Plain `Cmd+X` used to run code ("X for execute")
        // but in a text-editing app the OS-wide *cut* has to win: it is
        // now `CutBlock` in Normal (view) mode and native text cut in
        // Insert, so it never reaches RunCodeBlock.
        // The TUI uses the `g x` chord which lives in
        // `outl-tui/input/` for now — `Cmd` doesn't exist in
        // crossterm so the catalog can't drive both surfaces with a
        // single binding.
        Binding::new(
            shift_meta_ch('x'),
            Global,
            Action::RunCodeBlock,
            "Run code block (Cmd+Shift+X)",
        ),
        Binding::new(
            pair('g', 'x'),
            Normal,
            Action::RunCodeBlock,
            // TUI falls back to opening the markdown link under the
            // cursor when the block isn't code (issue #183).
            "Run code block / open link (TUI chord)",
        ),
        // ── Inline markdown wrappers (Insert mode — textarea focused) ──
        //
        // Mirrors the convention every popular markdown editor
        // (Notion, Obsidian, Discord, Slack, Typora) ships.
        // Cmd+B/I/E/K for bold / italic / code / link; Cmd+Shift+X
        // for strikethrough (Slack/Discord convention, avoids the
        // Cmd+Shift+S "Save As" conflict).
        Binding::new(meta('b'), Insert, Action::WrapBold, "Bold (**…**)"),
        // outl ships `_…_` as the canonical italic — the parser
        // accepts `*…*` too but `.md` projections emit underscores.
        Binding::new(meta('i'), Insert, Action::WrapItalic, "Italic (_…_)"),
        Binding::new(meta('e'), Insert, Action::WrapCode, "Inline code (`…`)"),
        Binding::new(
            meta('k'),
            Insert,
            Action::InsertLink,
            "Insert link ([label](url))",
        ),
        Binding::new(
            shift_meta_ch('x'),
            Insert,
            Action::WrapStrike,
            "Strikethrough (~~…~~)",
        ),
        // ── Normal mode (vim-style, TUI parity) ───────────────────
        Binding::new(ch('t'), Normal, Action::OpenToday, "Open today's journal"),
        Binding::new(
            key(Key::Home),
            Normal,
            Action::OpenToday,
            "Open today (Home)",
        ),
        Binding::new(ch('['), Normal, Action::PrevDay, "Previous journal day"),
        Binding::new(ch(']'), Normal, Action::NextDay, "Next journal day"),
        Binding::new(
            pair('g', 'j'),
            Normal,
            Action::OpenToday,
            "Jump to today (chord)",
        ),
        // `g d` = delete the focused page. "go delete" mnemonic, same
        // `g<action>` pattern as `g j` (today) / `g x` (execute) /
        // `g p` (pin). Acts on the sidebar-highlighted row when the
        // sidebar has focus (TUI) or the current page otherwise
        // (desktop / TUI outline). Each client confirms before
        // invoking — see `Action::DeletePage`.
        Binding::new(
            pair('g', 'd'),
            Normal,
            Action::DeletePage,
            "Delete page (chord)",
        ),
        // ── Reminders ──────────────────────────────────────────
        //
        // `g r` / `g R` follow the `g<action>` family (`g j`, `g x`,
        // `g d`). The issue's original sketch put the overlay on
        // `Ctrl+R`, but that is **already Redo** in Normal mode — and
        // a terminal cannot tell `Ctrl+R` from `Ctrl+Shift+R`, so the
        // TUI gets `g n` ("go notifications") and only the desktop
        // takes the `Cmd/Ctrl+Shift+R` spelling.
        Binding::new(
            pair('g', 'r'),
            Normal,
            Action::InsertRemind,
            "Add a reminder to this block (chord)",
        ),
        Binding::new(
            pair_shift_second('g', 'r'),
            Normal,
            Action::InsertRemindNag,
            "Nag me: remind:: now every 1h until DONE",
        ),
        Binding::new(
            pair('g', 'n'),
            Normal,
            Action::OpenReminders,
            "Open reminders (chord)",
        ),
        // `g s` snoozes the selected block's reminder an hour without
        // opening the list. The action had a desktop handler and no
        // chord at all, so it was unreachable everywhere.
        Binding::new(
            pair('g', 's'),
            Normal,
            Action::SnoozeReminder,
            "Snooze this block's reminder 1h",
        ),
        Binding::new(
            shift_meta_ch('r'),
            Global,
            Action::OpenReminders,
            "Open reminders (Cmd+Shift+R)",
        ),
        Binding::new(
            shift_ctrl_ch('r'),
            Global,
            Action::OpenReminders,
            "Open reminders (Ctrl+Shift+R)",
        ),
        // `Cmd+R` only — `Ctrl+R` stays Redo on Linux / Windows.
        Binding::new(
            meta('r'),
            Global,
            Action::InsertRemind,
            "Add a reminder (Cmd+R)",
        ),
        // ── Properties (`key:: value`) ─────────────────────────
        //
        // `Normal`, not `Global`: the editor writes onto the
        // *selected* block, and inside a textarea the selection is the
        // caret, not a block. A Global row would also `preventDefault`
        // a chord some webview / OS input method may want. The desktop
        // reaches Normal through its "nothing focused" detection, so
        // this fires in view mode with or without `vim_mode` — the
        // same rule `CutBlock` / `CopyBlock` follow.
        //
        // Dual-spelled per OS: the desktop adapter never rewrites
        // Cmd↔Ctrl, so a single META row dead-keys on Linux/Windows.
        Binding::new(
            shift_meta_ch('p'),
            Normal,
            Action::AddProperty,
            "Add a property to this block (Cmd+Shift+P)",
        ),
        Binding::new(
            shift_ctrl_ch('p'),
            Normal,
            Action::AddProperty,
            "Add a property to this block (Ctrl+Shift+P)",
        ),
        // `g p` = "go properties", the `g<action>` family again. The
        // TUI's overlay is the discoverable half of `:prop`: `:prop`
        // needs you to know the key, this lists what is already there.
        //
        // It **took** `g p` from the pin toggle, which moved to `g P`.
        // Pinning is a once-per-page act and already has `/pin`;
        // editing properties is a daily one, and `pinned::` is itself
        // a page property this overlay now edits. The shift also
        // matches `g r` / `g R`.
        Binding::new(
            pair('g', 'p'),
            Normal,
            Action::OpenProperties,
            "Open the property editor (chord)",
        ),
        Binding::new(
            pair_shift_second('g', 'p'),
            Normal,
            Action::TogglePin,
            "Toggle pinned:: on this page (chord)",
        ),
        Binding::new(
            ctrl('p'),
            Normal,
            Action::OpenPicker,
            "Quick switcher (fuzzy)",
        ),
        Binding::new(ch('?'), Normal, Action::ToggleHelp, "Toggle help popup"),
        Binding::new(
            ch(':'),
            Normal,
            Action::OpenCommandPalette,
            "Command palette",
        ),
        Binding::new(pair('q', 'q'), Normal, Action::Quit, "Quit (chord)"),
        // Vim's `ZZ` — "save and quit". outl auto-commits Insert on
        // every Normal-mode boundary, so by the time the chord
        // resolves the buffer is already on disk. Effectively `qq`
        // with a different muscle memory; both stay alive so users
        // arriving from vim don't trip.
        Binding::new(
            shift_pair('z', 'z'),
            Normal,
            Action::Quit,
            "Save and quit (vim ZZ chord)",
        ),
        Binding::new(ch('j'), Normal, Action::SelectionDown, "Selection down"),
        Binding::new(
            key(Key::Down),
            Normal,
            Action::SelectionDown,
            "Selection down",
        ),
        Binding::new(ch('k'), Normal, Action::SelectionUp, "Selection up"),
        Binding::new(key(Key::Up), Normal, Action::SelectionUp, "Selection up"),
        Binding::new(
            ch('i'),
            Normal,
            Action::EnterInsert,
            "Insert at end of block",
        ),
        Binding::new(
            shift_ch('i'),
            Normal,
            Action::EnterInsertAtStart,
            "Insert at start",
        ),
        // Vim's `a` — Insert one char past the cursor ("append").
        // Clamps at end of buffer so `a` at end-of-line behaves
        // like `i` there (no off-by-one cursor past the buffer).
        Binding::new(
            ch('a'),
            Normal,
            Action::EnterInsertAfter,
            "Insert after cursor (append)",
        ),
        // Vim `A` — append at end of block.
        Binding::new(
            shift_ch('a'),
            Normal,
            Action::EnterInsertAtEnd,
            "Insert at end of block (append)",
        ),
        // ── Vim char / line ops in Normal mode ────────────────────
        Binding::new(
            ch('x'),
            Normal,
            Action::DeleteCharUnderCursor,
            "Delete char under cursor",
        ),
        Binding::new(
            shift_ch('x'),
            Normal,
            Action::DeleteCharBeforeCursor,
            "Delete char before cursor",
        ),
        Binding::new(
            shift_ch('d'),
            Normal,
            Action::DeleteToEndOfBlock,
            "Delete to end of block",
        ),
        Binding::new(
            shift_ch('c'),
            Normal,
            Action::ChangeToEndOfBlock,
            "Change to end of block",
        ),
        Binding::new(
            shift_ch('s'),
            Normal,
            Action::SubstituteBlock,
            "Substitute block (clear + Insert)",
        ),
        Binding::new(ch('s'), Normal, Action::SubstituteChar, "Substitute char"),
        Binding::new(
            ch('r'),
            Normal,
            Action::ReplaceChar,
            "Replace char (arms r{ch})",
        ),
        Binding::new(
            ch('f'),
            Normal,
            Action::FindCharForward,
            "Find char forward (arms f{ch})",
        ),
        Binding::new(
            shift_ch('f'),
            Normal,
            Action::FindCharBackward,
            "Find char backward (arms F{ch})",
        ),
        Binding::new(
            ch('~'),
            Normal,
            Action::ToggleCharCase,
            "Toggle case of char under cursor",
        ),
        Binding::new(
            shift_ch('y'),
            Normal,
            Action::YankCurrentBlock,
            "Yank current block (Y alias of yy)",
        ),
        Binding::new(ch('e'), Normal, Action::CursorWordEnd, "Cursor to word end"),
        Binding::new(
            ch('*'),
            Normal,
            Action::SearchWordForward,
            "Search word under cursor (forward)",
        ),
        Binding::new(
            ch('#'),
            Normal,
            Action::SearchWordBackward,
            "Search word under cursor (backward)",
        ),
        // Fold-control chord family.
        Binding::new(pair('z', 'R'), Normal, Action::UnfoldAll, "Unfold all (zR)"),
        Binding::new(pair('z', 'M'), Normal, Action::FoldAll, "Fold all (zM)"),
        Binding::new(
            pair('z', 'z'),
            Normal,
            Action::CenterViewport,
            "Center viewport on cursor (zz)",
        ),
        // Zoom / focus chord family. `z i` descends into the selected
        // block (it becomes the outline root); `z o` pops back up one
        // level. Same `z*` namespace as the view ops above — zoom is
        // viewport state, not editing.
        Binding::new(
            pair('z', 'i'),
            Normal,
            Action::ZoomIn,
            "Zoom in on block (zi)",
        ),
        Binding::new(pair('z', 'o'), Normal, Action::ZoomOut, "Zoom out (zo)"),
        // Visual re-select + range indent / outdent.
        Binding::new(
            pair('g', 'v'),
            Normal,
            Action::ReselectLastVisual,
            "Reselect last Visual range (gv)",
        ),
        Binding::new(
            ch('>'),
            Visual,
            Action::IndentVisualRange,
            "Indent visual range",
        ),
        Binding::new(
            ch('<'),
            Visual,
            Action::OutdentVisualRange,
            "Outdent visual range",
        ),
        Binding::new(
            key(Key::Enter),
            Normal,
            Action::OpenRefUnderCursor,
            "Open ref / enter Insert",
        ),
        Binding::new(ch('o'), Normal, Action::NewBlockBelow, "New block below"),
        // View-mode counterpart of the Insert-mode `Cmd+Shift+Enter`
        // (`CommitAndContinue`): in Normal mode there's no in-flight
        // edit to commit, so it just creates the sibling below and
        // drops into Insert on it — same `NewBlockBelow` action as vim
        // `o`, but reachable without `vim_mode` (the desktop falls into
        // Normal dispatch whenever no textarea is focused). Distinct
        // mode from the Insert binding, so `lookup`'s mode-specific
        // resolution keeps both: Insert → `CommitAndContinue`, Normal →
        // `NewBlockBelow`.
        //
        // Two physical chords for the same gesture: `Cmd+Shift+Enter`
        // on macOS, `Ctrl+Shift+Enter` on Windows / Linux. The desktop
        // adapter keeps META and CTRL as distinct bits (it never
        // rewrites `Cmd`↔`Ctrl`), so each OS needs its own row — the
        // same two-row pattern `Cmd/Ctrl+P` and `Cmd/Ctrl+Z` already
        // use.
        Binding::new(
            shift_meta_key(Key::Enter),
            Normal,
            Action::NewBlockBelow,
            "New block below (Cmd+Shift+Enter)",
        ),
        Binding::new(
            shift_ctrl_key(Key::Enter),
            Normal,
            Action::NewBlockBelow,
            "New block below (Ctrl+Shift+Enter)",
        ),
        Binding::new(
            shift_ch('o'),
            Normal,
            Action::NewBlockAbove,
            "New block above",
        ),
        Binding::new(key(Key::Tab), Normal, Action::IndentBlock, "Indent block"),
        Binding::new(
            shift(Key::Tab),
            Normal,
            Action::OutdentBlock,
            "Outdent block",
        ),
        Binding::new(
            pair('d', 'd'),
            Normal,
            Action::DeleteBlock,
            "Delete block (chord)",
        ),
        // ── Block move + clipboard (Normal / view mode) ───────────
        //
        // Reorder the selected block among its siblings with
        // `Cmd+Shift+↑/↓` (Notion / Logseq muscle memory), and
        // cut / copy / paste a whole block + its subtree with the
        // OS-native `Cmd+X/C/V`. These are **Normal-mode** bindings
        // so they never shadow the native text cut / copy / paste
        // inside a block editor (Insert mode, where the chord isn't
        // in the catalog and the keystroke reaches the textarea).
        //
        // Each is dual-spelled per OS: META (`Cmd`) and CTRL, bound
        // twice because the desktop adapter never rewrites
        // `Cmd`↔`Ctrl` — the same two-row pattern `Cmd/Ctrl+Z` and
        // `Cmd/Ctrl+Shift+Enter` use. Without the CTRL row these
        // chords fire on macOS only and dead-key on Linux / Windows.
        Binding::new(
            shift_meta_key(Key::Up),
            Normal,
            Action::MoveBlockUp,
            "Move block up (Cmd+Shift+Up)",
        ),
        Binding::new(
            shift_ctrl_key(Key::Up),
            Normal,
            Action::MoveBlockUp,
            "Move block up (Ctrl+Shift+Up)",
        ),
        Binding::new(
            shift_meta_key(Key::Down),
            Normal,
            Action::MoveBlockDown,
            "Move block down (Cmd+Shift+Down)",
        ),
        Binding::new(
            shift_ctrl_key(Key::Down),
            Normal,
            Action::MoveBlockDown,
            "Move block down (Ctrl+Shift+Down)",
        ),
        Binding::new(meta('x'), Normal, Action::CutBlock, "Cut block (Cmd+X)"),
        Binding::new(ctrl('x'), Normal, Action::CutBlock, "Cut block (Ctrl+X)"),
        Binding::new(meta('c'), Normal, Action::CopyBlock, "Copy block (Cmd+C)"),
        Binding::new(ctrl('c'), Normal, Action::CopyBlock, "Copy block (Ctrl+C)"),
        Binding::new(meta('v'), Normal, Action::PasteBlock, "Paste block (Cmd+V)"),
        Binding::new(
            ctrl('v'),
            Normal,
            Action::PasteBlock,
            "Paste block (Ctrl+V)",
        ),
        // `Esc` in view mode cancels a pending cut (snaps the dimmed
        // block back). Reuses `ExitInsert` — a no-op blur otherwise,
        // since Normal mode has no focused textarea.
        Binding::new(
            key(Key::Esc),
            Normal,
            Action::ExitInsert,
            "Cancel pending cut",
        ),
        Binding::new(ch('c'), Normal, Action::ToggleCollapsed, "Fold / unfold"),
        Binding::new(
            pair('y', 'r'),
            Normal,
            Action::CopyBlockRef,
            "Copy block ref handle",
        ),
        Binding::new(ch('v'), Normal, Action::EnterVisual, "Enter Visual mode"),
        // Non-vim multi-select entry: `Shift+↓` / `Shift+↑` start a
        // contiguous block selection (anchored at the current block)
        // and grow it. Bound in Normal so the no-vim-mode user reaches
        // it via the DOM "nothing focused" → Normal fallback; it flips
        // the client into Visual, where the same chords keep extending.
        Binding::new(
            shift(Key::Down),
            Normal,
            Action::SelectRangeDown,
            "Select range down",
        ),
        Binding::new(
            shift(Key::Up),
            Normal,
            Action::SelectRangeUp,
            "Select range up",
        ),
        Binding::new(ch('u'), Normal, Action::Undo, "Undo"),
        Binding::new(ctrl('r'), Normal, Action::Redo, "Redo"),
        // OS-standard undo / redo chords (desktop). Deliberately
        // **Normal**, not Global: with a textarea focused the chord
        // must fall through to the webview (in-flight draft editing
        // is the textarea's own undo domain), and a Global binding
        // would `preventDefault` it away. Outside a textarea they
        // revert / re-apply the last committed block mutation.
        // `Ctrl` variants cover Windows / Linux (same pattern as the
        // `Cmd+P` / `Ctrl+P` pair above).
        Binding::new(meta('z'), Normal, Action::Undo, "Undo (Cmd+Z)"),
        Binding::new(ctrl('z'), Normal, Action::Undo, "Undo (Ctrl+Z)"),
        Binding::new(
            shift_meta_ch('z'),
            Normal,
            Action::Redo,
            "Redo (Cmd+Shift+Z)",
        ),
        Binding::new(
            shift_ctrl_ch('z'),
            Normal,
            Action::Redo,
            "Redo (Ctrl+Shift+Z)",
        ),
        // ── Insert mode ───────────────────────────────────────────
        //
        // Only `Esc` and `Cmd+Enter` land in the catalog. Every
        // other in-editor chord (`Tab`/`Shift-Tab` for indent,
        // `Backspace` on an empty block, `[[`/`((` auto-pair,
        // plain `Enter` for a newline) is owned by `<BlockRow />`'s
        // textarea `onKeyDown`. If we bound them here, the
        // dispatcher would `preventDefault` first and the textarea
        // would never see the keystroke.
        Binding::new(
            key(Key::Esc),
            Insert,
            Action::ExitInsert,
            "Commit + exit Insert",
        ),
        Binding::new(
            shift_meta_key(Key::Enter),
            Insert,
            Action::CommitAndContinue,
            "Commit + new block below",
        ),
        Binding::new(
            shift_ctrl_key(Key::Enter),
            Insert,
            Action::CommitAndContinue,
            "Commit + new block below (Ctrl+Shift+Enter)",
        ),
        // ── Visual mode ──────────────────────────────────────────
        Binding::new(key(Key::Esc), Visual, Action::ExitInsert, "Leave Visual"),
        Binding::new(ch('y'), Visual, Action::YankRange, "Yank range"),
        Binding::new(ch('d'), Visual, Action::DeleteRange, "Delete range"),
        // `Delete` / `Backspace` delete the range too — the vim-free
        // way to erase a multi-selection (a non-vim user never reaches
        // for `d`). The TUI keeps `d` / `x`; these are the desktop's
        // standard-keyboard spellings of the same batch delete.
        Binding::new(
            key(Key::Delete),
            Visual,
            Action::DeleteRange,
            "Delete range (Delete)",
        ),
        Binding::new(
            key(Key::Backspace),
            Visual,
            Action::DeleteRange,
            "Delete range (Backspace)",
        ),
        Binding::new(ch('j'), Visual, Action::SelectionDown, "Extend down"),
        Binding::new(ch('k'), Visual, Action::SelectionUp, "Extend up"),
        // `Shift+↓` / `Shift+↑` keep extending once selection mode is
        // active (the non-vim entry chord stays consistent inside the
        // range).
        Binding::new(
            shift(Key::Down),
            Visual,
            Action::SelectRangeDown,
            "Extend down (Shift+Down)",
        ),
        Binding::new(
            shift(Key::Up),
            Visual,
            Action::SelectRangeUp,
            "Extend up (Shift+Up)",
        ),
        // Reorder the whole range among its siblings. Mirrors the
        // single-block `Cmd/Ctrl+Shift+↑/↓` (Normal) but acts on every
        // block in the Visual range; dual-spelled per OS because the
        // desktop adapter never rewrites `Cmd`↔`Ctrl`.
        Binding::new(
            shift_meta_key(Key::Up),
            Visual,
            Action::MoveVisualRangeUp,
            "Move range up (Cmd+Shift+Up)",
        ),
        Binding::new(
            shift_ctrl_key(Key::Up),
            Visual,
            Action::MoveVisualRangeUp,
            "Move range up (Ctrl+Shift+Up)",
        ),
        Binding::new(
            shift_meta_key(Key::Down),
            Visual,
            Action::MoveVisualRangeDown,
            "Move range down (Cmd+Shift+Down)",
        ),
        Binding::new(
            shift_ctrl_key(Key::Down),
            Visual,
            Action::MoveVisualRangeDown,
            "Move range down (Ctrl+Shift+Down)",
        ),
        // ── Overlay (picker / palette have their own keys; arrow + Enter + Esc) ──
        Binding::new(key(Key::Esc), Overlay, Action::ExitInsert, "Close overlay"),
        Binding::new(
            key(Key::Down),
            Overlay,
            Action::SelectionDown,
            "Highlight next",
        ),
        Binding::new(
            key(Key::Up),
            Overlay,
            Action::SelectionUp,
            "Highlight previous",
        ),
    ]
}
