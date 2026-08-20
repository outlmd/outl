//! Every named editor operation outl can perform in response to a
//! key chord. Adding a variant is a coordinated change: each client
//! must learn to dispatch it (TUI: a method on `App`; desktop: a
//! JS handler that calls the right Tauri command).
//!
//! Variants are grouped by intent — chrome, navigation, editing,
//! visual / range, code execution — so the help overlay can list
//! them in sensible sections.

use serde::{Deserialize, Serialize};

/// Tagged union of every action the binding catalog can resolve to.
///
/// `#[serde(tag = "kind")]` keeps the wire format readable
/// (`{"kind":"OpenToday"}`) so the desktop frontend can `switch`
/// on a string discriminant instead of arbitrary indices.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Action {
    // ── chrome ────────────────────────────────────────────────────
    /// Open the picker overlay (Cmd/Ctrl+P).
    OpenPicker,
    /// Open the slash / colon command palette.
    OpenCommandPalette,
    /// Toggle the help overlay.
    ToggleHelp,
    /// Toggle the left sidebar.
    ToggleSidebar,
    /// Toggle the right backlinks panel.
    ToggleBacklinks,
    /// Open the settings modal.
    OpenSettings,
    /// Quit the application (TUI: `q q` chord; desktop: `Cmd+Q` via OS).
    Quit,

    // ── navigation ───────────────────────────────────────────────
    /// Open today's journal.
    OpenToday,
    /// Open the previous day's journal (only meaningful on a journal page).
    PrevDay,
    /// Open the next day's journal.
    NextDay,
    /// Selection one block down (in flat DFS order, skipping collapsed subtrees).
    SelectionDown,
    /// Selection one block up.
    SelectionUp,
    /// Open the ref under the cursor (`[[ref]]`, `#tag`, `((blk-…))`).
    /// In Normal mode where the cursor sits on a block, falls through
    /// to "enter Insert at end of block" when there's no ref under cursor.
    OpenRefUnderCursor,

    // ── block structure ──────────────────────────────────────────
    /// Enter Insert at end of current block (TUI `i` / `Enter` on Normal).
    EnterInsert,
    /// Enter Insert at start of current block (TUI `I`).
    EnterInsertAtStart,
    /// Enter Insert one char past the cursor (vim's `a` — "append"). Clamps at
    /// end of buffer so `a` at the last position behaves like `i`.
    EnterInsertAfter,
    /// Enter Insert with the cursor jumped to end of block (vim's `A`).
    EnterInsertAtEnd,
    /// Delete the char under the cursor in Normal mode (vim `x`).
    DeleteCharUnderCursor,
    /// Delete the char before the cursor in Normal mode (vim `X`).
    DeleteCharBeforeCursor,
    /// Delete from cursor to end of block (vim `D` / `d$`).
    DeleteToEndOfBlock,
    /// `D` + enter Insert at the new EOL (vim `C` / `c$`).
    ChangeToEndOfBlock,
    /// Clear the current block's text and enter Insert at column 0
    /// (vim `S` / `cc`).
    SubstituteBlock,
    /// Delete the char under cursor and enter Insert (vim `s` / `xi`).
    SubstituteChar,
    /// Replace the char under the cursor with the next typed char
    /// without entering Insert (vim `r{ch}`). The TUI implements this
    /// via a one-shot "pending input op" — the desktop wires it to a
    /// modal prompt or simply doesn't expose it outside vim_mode.
    ReplaceChar,
    /// Find next occurrence of the next typed char on the current
    /// block, forward (vim `f{ch}`).
    FindCharForward,
    /// Find previous occurrence of the next typed char (vim `F{ch}`).
    FindCharBackward,
    /// Toggle the case of the char under the cursor and advance one
    /// position (vim `~`).
    ToggleCharCase,
    /// Move the cursor to the end of the current / next word
    /// (vim `e`). Distinct from `w` which lands at word *start*.
    CursorWordEnd,
    /// Unfold every block on the current page (vim `zR`).
    UnfoldAll,
    /// Fold every block on the current page (vim `zM`).
    FoldAll,
    /// Center the viewport vertically on the selected block
    /// (vim `zz`).
    CenterViewport,
    /// Zoom in: make the selected block the root of the outline view,
    /// hiding everything outside its subtree (vim `z i`; desktop
    /// `Cmd/Ctrl+Shift+]`). Pure local view state — never an `Op`.
    ZoomIn,
    /// Zoom out: pop one level back up the zoom stack toward the full
    /// page (vim `z o`; desktop `Cmd/Ctrl+Shift+[`).
    ZoomOut,
    /// Search the workspace for the word under cursor, forward
    /// (vim `*`). `n` / `N` then walk through the results.
    SearchWordForward,
    /// Same as `SearchWordForward` but backward (vim `#`).
    SearchWordBackward,
    /// Re-enter Visual mode at the last captured range (vim `gv`).
    ReselectLastVisual,
    /// Indent every block in the Visual range (vim `>` in Visual).
    IndentVisualRange,
    /// Outdent every block in the Visual range (vim `<` in Visual).
    OutdentVisualRange,
    /// New block below + Insert (TUI `o`; desktop `Enter` in Insert).
    NewBlockBelow,
    /// New block above + Insert (TUI `O`).
    NewBlockAbove,
    /// Indent the current block (`Tab`).
    IndentBlock,
    /// Outdent the current block (`Shift-Tab`).
    OutdentBlock,
    /// Move the current block up among its siblings.
    MoveBlockUp,
    /// Move the current block down.
    MoveBlockDown,
    /// Delete the current block (TUI `d d` chord; desktop `Backspace` on empty).
    DeleteBlock,
    /// Fold / unfold the current block.
    ToggleCollapsed,
    /// Cycle task state (none → TODO → DOING → DONE → none).
    ToggleTodo,
    /// Copy the current block's `((blk-…))` ref handle to clipboard.
    CopyBlockRef,

    // ── page operations ──────────────────────────────────────────
    //
    // Page-level actions that act on the page as a whole rather than
    // an individual block. Wired through the shared `(chord, action)`
    // catalog so every client agrees on the spelling: `g d` (Normal
    // mode, "go delete") is the canonical chord, mirroring the
    // `g<action>` family (`g j`, `g x`, `g p`). Desktop also exposes
    // it via the sidebar hover `×` button; mobile via long-press in
    // the page switcher (no chord consumed). Each client confirms
    // before invoking the underlying `outl_actions::page::delete`.
    /// Delete the focused page. When the sidebar has focus (TUI), the
    /// highlighted row is the target; otherwise the current page.
    /// Each client asks for confirmation before invoking
    /// `outl_actions::page::delete`.
    DeletePage,

    // ── reminders (`remind::`) ───────────────────────────────────
    //
    // Authoring the rule and inspecting the schedule are chorded on
    // every client; *delivering* the notification is per-OS and never
    // a chord. Every client delivers, the TUI included (OSC 9 plus a
    // toast on its event-loop tick). What a terminal session can't do
    // is deliver while it's closed, having no background presence.
    /// Insert a `remind:: ` skeleton as a property of the selected
    /// block and put the cursor after it, ready to type a time.
    InsertRemind,
    /// Insert the "nag me" preset — `remind:: now every 1h until DONE`
    /// — in one chord, for the TODO you must not drop today.
    InsertRemindNag,
    /// Open the Reminders surface: every block in the workspace with a
    /// `remind::`, grouped by next fire. TUI overlay, desktop modal,
    /// mobile tab.
    OpenReminders,
    /// Snooze the selected reminder by one hour
    /// (`Op::SnoozeRemind`, so it silences every device).
    SnoozeReminder,

    // ── properties (`key:: value`) ───────────────────────────────
    //
    // `remind::` above is one property with its own chords; this is
    // the generic door. Creating the *first* property on a block was
    // impossible in both GUI clients (issue #13) — you could edit a
    // chip that already existed and nothing more.
    /// Open a blank property editor on the selected block: an empty
    /// key field (completing from the workspace's key catalogue) and
    /// an empty value.
    AddProperty,
    /// Open the property editor for the selected block: every
    /// `key:: value` already on it, plus the affordances to add,
    /// change and remove one.
    ///
    /// Distinct from [`Action::AddProperty`], which skips straight to
    /// a blank pair. The TUI needs both doors because its overlay is
    /// list-first (`g p` lists, `o` inside adds); a client whose
    /// property surface is always visible (the desktop's chip row) may
    /// reasonably bind only `AddProperty`.
    OpenProperties,
    /// Toggle `pinned:: true` on the current page.
    ///
    /// TUI-only today. It lives in the catalog because it holds a
    /// chord (`g P`) in the `g<action>` family, and a chord outside
    /// the catalog is a chord the duplicate-detection test cannot see
    /// — which is how it ended up colliding with `g p` in the first
    /// place.
    TogglePin,

    // ── block clipboard (view-mode cut / copy / paste of a block) ─
    //
    // These act on a whole block + its subtree while the user is in
    // view (Normal) mode — distinct from the OS-native text cut /
    // copy / paste that fires inside a block editor (Insert mode).
    /// Cut the selected block + subtree to the block clipboard,
    /// marked to **move by id** (desktop `Cmd+X` in view mode). The
    /// matching paste emits a single `Op::Move`, so the block keeps
    /// its identity and every `((blk-…))` ref / backlink stays valid.
    CutBlock,
    /// Copy the selected block + subtree to the block clipboard as
    /// markdown (desktop `Cmd+C` in view mode). The matching paste
    /// duplicates the subtree with fresh ids.
    CopyBlock,
    /// Paste the block clipboard as the sibling after the selected
    /// block (desktop `Cmd+V` in view mode). A cut clipboard moves
    /// the original node; a copy clipboard duplicates it.
    PasteBlock,

    // ── insert-mode commits ──────────────────────────────────────
    /// Commit in-flight edit and leave Insert mode (`Esc`).
    ExitInsert,
    /// Commit + new block below + continue editing (`Enter` in Insert).
    CommitAndContinue,
    /// Backspace on an empty block deletes the block (desktop in Insert).
    DeleteEmptyBlock,

    // ── visual / range ───────────────────────────────────────────
    /// Enter Visual mode (TUI `v`).
    EnterVisual,
    /// Yank the currently selected block to the register (vim `yy` / `Y`).
    YankCurrentBlock,
    /// Yank the visual range to register.
    YankRange,
    /// Delete the visual range.
    DeleteRange,
    /// Extend (or start, from Normal mode) the selection one block down.
    /// The non-vim multi-select entry: `Shift+Down` anchors at the
    /// current block if no range is active, then grows it downward.
    SelectRangeDown,
    /// Extend (or start) the selection one block up (`Shift+Up`).
    SelectRangeUp,
    /// Move every block in the Visual range up among its siblings.
    MoveVisualRangeUp,
    /// Move every block in the Visual range down among its siblings.
    MoveVisualRangeDown,

    // ── code execution ───────────────────────────────────────────
    /// Run the code block under the cursor through `outl-exec`.
    RunCodeBlock,

    // ── inline markdown wrappers (Insert mode only) ──────────────
    //
    // These mirror the conventions every popular markdown editor
    // ships (Notion, Obsidian, Discord, Slack): chord wraps the
    // active textarea selection, or inserts the delimiter pair
    // around the caret. The handler accesses `document.activeElement`
    // — it does not need the workspace.
    /// Wrap selection (or insert empty pair) with `**bold**`.
    WrapBold,
    /// Wrap with `_italic_` (outl's canonical italic delimiter; the
    /// parser also accepts `*…*` for compatibility with mainstream
    /// markdown, but new content uses underscores).
    WrapItalic,
    /// Wrap with `` `code` ``.
    WrapCode,
    /// Wrap with `~~strike~~`.
    WrapStrike,
    /// Wrap selection as the label of `[label](url)` and select
    /// `url` for the user to type the destination into.
    InsertLink,

    // ── undo / redo ──────────────────────────────────────────────
    /// Undo the last mutation (`u` in TUI Normal; `Cmd+Z` desktop).
    Undo,
    /// Redo (`Ctrl+R` in TUI Normal; `Cmd+Shift+Z` desktop).
    Redo,
}
