# Client parity

What each client does with each [`Action`](shortcuts.md) in the shared catalog.

## Why this table is generated

Before this file existed, "does mobile have undo?" had three answers in three places, and they disagreed:

- [`docs/shortcuts.md`](shortcuts.md) listed `y r` (copy block ref) and `:` (command palette) as desktop chords.
  Neither has a handler — the keys do nothing and log to a console the user never opens.
- The same doc listed mobile undo / redo as "toolbar".
  Mobile has neither ([#14](https://github.com/outlmd/outl/issues/14) is still open), and `Journal.tsx` says so in a comment.
- The desktop dispatcher's own comment called its `console.warn` something "the user sees".

Three hand-maintained copies of one fact, each stale in a different direction, and nothing that could fail.
So the fact moved into the code — `crates/outl-shortcuts/src/support.rs` — and this table is a projection of it.

**Do not edit the table below by hand.**
It is regenerated from `outl_shortcuts::support` and pinned by `the_parity_doc_matches_the_code`:

```sh
OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts
```

## Why a `match`, and not a list

`support()` is an exhaustive `match` on `Action`.
A new variant **does not compile** until all three clients have declared what they do with it.

That is the whole mechanism.
The gap gets recorded when it is created, by the person creating it — instead of being discovered months later by a user pressing a key that does nothing.
It is [invariant 10](../CLAUDE.md) applied to features rather than authority: *when you add a capability, enumerate who does not have it.*

## How to read it

| Cell | Means |
|---|---|
| ✅ | The client performs the action. Chord, button or gesture — the user does not care which, so neither does this table. |
| ✅ _native_ | The platform performs it, not outl (`Backspace` in an empty textarea, the OS text clipboard). Reachable, but there is no handler to test for. |
| ⚠️ | Reachable, with less than the full semantics. The text is what the user is told. |
| ❌ | Not implemented here, and it should be. The text is what the user is told, so it says what to do instead — never "unimplemented". |
| — | Cannot exist on this client by construction. The text says why. |

The ⚠️ and ❌ text is not documentation.
It is the string the client shows when the user reaches for the action, so it is written for them and pinned by `nudges_are_written_for_the_user_not_the_developer`.

## What this table does not cover

Only actions in the chord catalog.
Features with no `Action` — page history, assets, the plugin marketplace, the calendar — diverge across clients the same way and are not yet tracked anywhere.
That is the next gap, not a closed one.

<!-- BEGIN GENERATED: client-parity -->

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| `OpenPicker` | ✅ | ✅ | ✅ |
| `OpenCommandPalette` | ✅ | ❌ The command palette isn't on the desktop yet — use the quick switcher (Cmd/Ctrl+P) or the slash menu inside a block. | ❌ The command palette isn't on mobile yet — use the slash menu inside a block. |
| `ToggleHelp` | ✅ | ✅ | ✅ |
| `ToggleSidebar` | ✅ | ✅ | — _Mobile is single-pane — the page switcher replaces the sidebar._ |
| `ToggleBacklinks` | ✅ | ✅ | ✅ |
| `OpenSettings` | ✅ | ✅ | ✅ |
| `Quit` | ✅ | ✅ | — _Mobile apps are backgrounded by the OS, not quit from inside._ |
| `OpenToday` | ✅ | ✅ | ✅ |
| `PrevDay` | ✅ | ✅ | ✅ |
| `NextDay` | ✅ | ✅ | ✅ |
| `SelectionDown` | ✅ | ✅ | — _Not on mobile — tap the block you want instead of moving a selection._ |
| `SelectionUp` | ✅ | ✅ | — _Not on mobile — tap the block you want instead of moving a selection._ |
| `OpenRefUnderCursor` | ✅ | ✅ | ✅ |
| `EnterInsert` | ✅ | ✅ | ✅ |
| `EnterInsertAtStart` | ✅ | ✅ | ✅ |
| `EnterInsertAfter` | ✅ | ⚠️ No character cursor on the desktop, so `a` behaves like `i`. | — _Mobile has no vim modes — it edits directly on tap._ |
| `EnterInsertAtEnd` | ✅ | ✅ | ✅ |
| `DeleteCharUnderCursor` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `DeleteCharBeforeCursor` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `DeleteToEndOfBlock` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `ChangeToEndOfBlock` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `SubstituteBlock` | ✅ | ✅ | — _Mobile has no vim modes — it edits directly on tap._ |
| `SubstituteChar` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `ReplaceChar` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `FindCharForward` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `FindCharBackward` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `ToggleCharCase` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `CursorWordEnd` | ✅ | ❌ This vim op needs a character cursor inside the block, which only the TUI has. Edit the block and use the arrow keys, or run it from the TUI. | — _Mobile has no vim modes — it edits directly on tap._ |
| `UnfoldAll` | ✅ | ✅ | ❌ Unfold-all isn't on mobile yet — tap each bullet to expand it. |
| `FoldAll` | ✅ | ✅ | ❌ Fold-all isn't on mobile yet — tap each bullet to collapse it. |
| `CenterViewport` | ✅ | ✅ | — _Mobile scrolls by touch — there is no cursor to centre on._ |
| `ZoomIn` | ✅ | ✅ | ✅ |
| `ZoomOut` | ✅ | ✅ | ✅ |
| `SearchWordForward` | ✅ | ⚠️ The desktop seeds the quick switcher instead of jumping between hits. | ❌ Search from the outline isn't on mobile yet (issue #19) — use the page switcher. |
| `SearchWordBackward` | ✅ | ⚠️ The desktop seeds the quick switcher instead of jumping between hits. | ❌ Search from the outline isn't on mobile yet (issue #19) — use the page switcher. |
| `ReselectLastVisual` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `IndentVisualRange` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `OutdentVisualRange` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `NewBlockBelow` | ✅ | ✅ | ✅ |
| `NewBlockAbove` | ✅ | ✅ | ❌ Only "new block below" is on mobile — add it below and drag it up. |
| `IndentBlock` | ✅ | ✅ | ✅ |
| `OutdentBlock` | ✅ | ✅ | ✅ |
| `MoveBlockUp` | ✅ | ✅ | ✅ |
| `MoveBlockDown` | ✅ | ✅ | ✅ |
| `DeleteBlock` | ✅ | ✅ | ✅ |
| `ToggleCollapsed` | ✅ | ✅ | ✅ |
| `ToggleTodo` | ✅ | ✅ | ✅ |
| `CopyBlockRef` | ✅ | ❌ Copying a block ref isn't on the desktop yet — open the block's properties, or copy the handle from the TUI with `y r`. | ❌ Copying a block ref isn't on mobile yet (issue #18). |
| `DeletePage` | ✅ | ✅ | ✅ |
| `InsertRemind` | ✅ | ✅ | ✅ |
| `InsertRemindNag` | ✅ | ✅ | ❌ The nag preset isn't on mobile — add a reminder, then edit the rule. |
| `OpenReminders` | ✅ | ✅ | ✅ |
| `SnoozeReminder` | ✅ | ✅ | ✅ |
| `AddProperty` | ✅ | ✅ | ✅ |
| `OpenProperties` | ✅ | ✅ | ✅ |
| `TogglePin` | ✅ | ❌ Pinning a page isn't on the desktop yet — pin it from the TUI with `g P`. | ❌ Pinning a page isn't on mobile yet — pin it from the TUI or desktop. |
| `CutBlock` | ❌ Cutting a whole block isn't in the TUI yet — use `d d`, then paste. | ✅ | ❌ Cutting a whole block isn't on mobile yet — drag it instead. |
| `CopyBlock` | ❌ Copying a whole block + subtree isn't in the TUI yet — use `y y` for the block alone. | ✅ | ❌ Copying a whole block isn't on mobile yet. |
| `PasteBlock` | ❌ Block-clipboard paste isn't in the TUI yet — `p` pastes the OS clipboard. | ✅ | ❌ Block-clipboard paste isn't on mobile yet. |
| `ExitInsert` | ✅ | ✅ | ✅ |
| `CommitAndContinue` | ✅ | ✅ | ✅ |
| `DeleteEmptyBlock` | ✅ | ✅ _native — Backspace in an empty textarea is handled by the editor itself._ | ❌ Backspace doesn't delete an empty block on mobile — swipe the row to delete it. |
| `EnterVisual` | ✅ | ⚠️ The desktop enters a range selection rather than a full modal Visual state. | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `YankCurrentBlock` | ✅ | ✅ | ❌ Yanking a block isn't on mobile yet. |
| `YankRange` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `DeleteRange` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `SelectRangeDown` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `SelectRangeUp` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `MoveVisualRangeUp` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `MoveVisualRangeDown` | ✅ | ✅ | ❌ Selecting a range of blocks isn't on mobile yet. Act on one block at a time, or use the desktop or TUI. |
| `RunCodeBlock` | ✅ | ✅ | ✅ |
| `WrapBold` | ✅ | ✅ | ✅ |
| `WrapItalic` | ✅ | ✅ | ✅ |
| `WrapCode` | ✅ | ✅ | ✅ |
| `WrapStrike` | ✅ | ✅ | ✅ |
| `InsertLink` | ✅ | ✅ | ✅ |
| `Undo` | ✅ | ✅ | ❌ Undo isn't on mobile yet (issue #14) — the change is already saved and syncs to your other devices. |
| `Redo` | ✅ | ✅ | ❌ Redo isn't on mobile yet (issue #14). |
<!-- END GENERATED: client-parity -->
