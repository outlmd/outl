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
It is [invariant 12](../CLAUDE.md): *when you add a capability, enumerate who does not have it.*

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
| `UnfoldAll` | ✅ | ✅ | ✅ |
| `FoldAll` | ✅ | ✅ | ✅ |
| `CenterViewport` | ✅ | ✅ | — _Mobile scrolls by touch — there is no cursor to centre on._ |
| `ZoomIn` | ✅ | ✅ | ✅ |
| `ZoomOut` | ✅ | ✅ | ✅ |
| `SearchWordForward` | ✅ | ⚠️ The desktop seeds the quick switcher instead of jumping between hits. | ⚠️ Mobile has no word-under-cursor to search from and no forward/backward stepping between hits — open the page switcher's Blocks tab (issue #19) and type the word instead. |
| `SearchWordBackward` | ✅ | ⚠️ The desktop seeds the quick switcher instead of jumping between hits. | ⚠️ Mobile has no word-under-cursor to search from and no forward/backward stepping between hits — open the page switcher's Blocks tab (issue #19) and type the word instead. |
| `ReselectLastVisual` | ✅ | ✅ | ✅ |
| `IndentVisualRange` | ✅ | ✅ | ✅ |
| `OutdentVisualRange` | ✅ | ✅ | ✅ |
| `NewBlockBelow` | ✅ | ✅ | ✅ |
| `NewBlockAbove` | ✅ | ✅ | ✅ |
| `IndentBlock` | ✅ | ✅ | ✅ |
| `OutdentBlock` | ✅ | ✅ | ✅ |
| `MoveBlockUp` | ✅ | ✅ | ✅ |
| `MoveBlockDown` | ✅ | ✅ | ✅ |
| `DeleteBlock` | ✅ | ✅ | ✅ |
| `ToggleCollapsed` | ✅ | ✅ | ✅ |
| `ToggleTodo` | ✅ | ✅ | ✅ |
| `CopyBlockRef` | ✅ | ❌ Copying a block ref isn't on the desktop yet — open the block's properties, or copy the handle from the TUI with `y r`. | ✅ |
| `DeletePage` | ✅ | ✅ | ✅ |
| `InsertRemind` | ✅ | ✅ | ✅ |
| `InsertRemindNag` | ✅ | ✅ | ❌ The nag preset isn't on mobile — add a reminder, then edit the rule. |
| `OpenReminders` | ✅ | ✅ | ✅ |
| `SnoozeReminder` | ✅ | ✅ | ✅ |
| `AddProperty` | ✅ | ✅ | ✅ |
| `OpenProperties` | ✅ | ✅ | ✅ |
| `TogglePin` | ✅ | ❌ Pinning a page isn't on the desktop yet — pin it from the TUI with `g P`. | ✅ |
| `CutBlock` | ❌ Cutting a whole block isn't in the TUI yet — use `d d`, then paste. | ✅ | ⚠️ Mobile's cut duplicates the block with a fresh id instead of moving it, so any ((blk-…)) refs pointing at it go stale — copy the ref first (long-press → "Copy block ref") if something else links to it. |
| `CopyBlock` | ❌ Copying a whole block + subtree isn't in the TUI yet — use `y y` for the block alone. | ✅ | ✅ |
| `PasteBlock` | ❌ Block-clipboard paste isn't in the TUI yet — `p` pastes the OS clipboard. | ✅ | ✅ |
| `ExitInsert` | ✅ | ✅ | ✅ |
| `CommitAndContinue` | ✅ | ✅ | ✅ |
| `DeleteEmptyBlock` | ✅ | ✅ _native — Backspace in an empty textarea is handled by the editor itself._ | ✅ |
| `EnterVisual` | ✅ | ⚠️ The desktop enters a range selection rather than a full modal Visual state. | ⚠️ Mobile enters a touch-native block selection (long-press a block, then "Select blocks") rather than a modal Visual state. |
| `YankCurrentBlock` | ✅ | ✅ | ✅ |
| `YankRange` | ✅ | ✅ | ✅ |
| `DeleteRange` | ✅ | ✅ | ✅ |
| `SelectRangeDown` | ✅ | ✅ | ✅ |
| `SelectRangeUp` | ✅ | ✅ | ✅ |
| `MoveVisualRangeUp` | ✅ | ✅ | ✅ |
| `MoveVisualRangeDown` | ✅ | ✅ | ✅ |
| `RunCodeBlock` | ✅ | ✅ | ✅ |
| `WrapBold` | ✅ | ✅ | ✅ |
| `WrapItalic` | ✅ | ✅ | ✅ |
| `WrapCode` | ✅ | ✅ | ✅ |
| `WrapStrike` | ✅ | ✅ | ✅ |
| `InsertLink` | ✅ | ✅ | ✅ |
| `Undo` | ✅ | ✅ | ✅ |
| `Redo` | ✅ | ✅ | ✅ |
<!-- END GENERATED: client-parity -->

## Capability parity

The table above covers only actions in the chord catalog — a chord fires a
handler, and `support()` is exhaustive over it.
Page history, the plugin marketplace, a calendar grid, templates, attaching a
file, and pairing a new device have no chord anywhere: they're a button, a
sheet, or a slash command, not a key.
That used to mean "not tracked anywhere" — [RFC 0253](rfcs/0253-client-capability-catalog.md)
closed it with a second exhaustive `match`, over `Capability` instead of
`Action`, reusing the same `Support` states and generated-table mechanism.

**Do not edit the table below by hand** — same rule, same regeneration
command:

```sh
OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts
```

A capability already covered by an `Action` — backlinks (`ToggleBacklinks`),
block properties (`OpenProperties` / `AddProperty`), reminders
(`OpenReminders` / `InsertRemind`) — stays in the table above and is
deliberately **not** duplicated here; a second entry for the same fact is
the drift this RFC exists to prevent.

<!-- BEGIN GENERATED: capability-parity -->

| Capability | TUI | Desktop | Mobile |
|---|---|---|---|
| `PageHistory` | ❌ Page history isn't in the TUI — open the same page in the desktop app and use the ⏱ button to see what the op log recorded. | ✅ | ❌ Page history isn't on mobile yet — open the same page in the desktop app and use the ⏱ button. |
| `PluginMarketplace` | ❌ Browsing and installing plugins from a marketplace isn't in the TUI — install by id from a terminal with `outl plugin install <id>`, or use the desktop or mobile app. | ✅ | ✅ |
| `Calendar` | ❌ There's no calendar grid here — open the quick switcher (Cmd/Ctrl+P) and type the date (YYYY-MM-DD) to jump straight to that journal page. | ❌ There's no calendar grid here — open the quick switcher (Cmd/Ctrl+P) and type the date (YYYY-MM-DD) to jump straight to that journal page. | ✅ |
| `Templates` | ✅ | ✅ | ✅ |
| `Assets` | ✅ | ✅ | ✅ |
| `PeerPairing` | ❌ Pairing a new device isn't in the TUI — run `outl peer pair` or `outl peer qr` from a terminal, or pair from the desktop or mobile app. | ⚠️ The desktop can host a pairing (show the QR / ticket) but has no camera to scan one — to join an existing workspace from a desktop, run `outl peer pair` in a terminal. | ✅ |
| `ReminderNotificationActions` | ❌ Reminder banners can't carry buttons here — open the reminders list (Ctrl+R in the TUI, Cmd/Ctrl+Shift+R on desktop) to snooze or tick off what came due. | ❌ Reminder banners can't carry buttons here — open the reminders list (Ctrl+R in the TUI, Cmd/Ctrl+Shift+R on desktop) to snooze or tick off what came due. | ✅ |
<!-- END GENERATED: capability-parity -->

## What this table does not cover

Both tables cover the three clients that render an outline — TUI, desktop,
mobile.
The CLI and the MCP server are deliberately out of scope: they expose
operations (`outl search`, `outl_batch`, …), not user-facing capabilities, so
the gap between them and the GUI clients is a vocabulary mismatch rather than
a missing feature.
That is sub-project D of the client convergence effort
([#255](https://github.com/outlmd/outl/issues/255)), not a closed gap and not
this document's job.
