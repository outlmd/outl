# How the desktop resolves a keystroke

The desktop fetches the chord catalog once and resolves every keystroke in TypeScript.
This is where each group of chords is decided: the handler assembly, keyboard reach for the chrome, undo / redo, inline markdown, paste, the block editor, vim parity, and `Enter` outside a textarea.

The chord list itself is [`docs/shortcuts.md`](shortcuts.md), which is generated from `outl-shortcuts`.
Nothing here duplicates it.

Moved out of [`crates/outl-desktop/CLAUDE.md`](../crates/outl-desktop/CLAUDE.md), which is loaded into the model's context on every session in that crate.
These sections are reference: you read one when you touch that part of the client.
The rules that hold whatever you are editing stayed in `CLAUDE.md`.

## `action-handlers.ts` is eight files

It was 1,038 lines, and it sat at its recorded size baseline, so nothing could be added to it.
It is the **assembly** now: build the context, spread the groups, carry the "deliberately absent" note.

The split axis is **what a keystroke changes**, not what kind of code the handler is.
A new handler goes in the sibling that owns its answer to that question, never back in the parent.
The siblings and the one-line responsibility of each are in the [layout tree](development.md#desktop-crate-layout); each file's header doc carries the reasoning for its own boundary.

Three things the split decided on purpose:

- **Only `buildHandlers` is exported from the parent.**
  `AppShell` and every test that reaches the map import it and nothing else, so there were no import paths to update.
- **The split moved declarations, and a pass after it deduplicated the bodies.**
  Two passes on purpose: a declaration move is reviewable on sight, a body change is not.
  Same net for both: `shortcuts.support.test.ts` pins the **presence** of all 66 keys in both directions, and nothing about what one does.
  That is why `clipboard-handlers.test.ts` exists for the two it never reached, `YankCurrentBlock` and `YankRange`.
  `clipboard-handlers.refer.test.ts` is the same gap for `CopyBlockRef` and `CopyBlockEmbed`.
  Both take one block id and answer one string, so a handler calling the other's command copies something that looks right and silently is not.
  The dedup collapsed eight inline copies of `ctx.runOn`, three copies of one batch loop (`ctx.runEach` owns it now), the `Esc` cascade's second list of panels, and seven toggles written two ways.
  `store.togglePanel` owns that flip now, including the chrome buttons, which were a list of their own.
- **`exitVisual` still writes `mode: "vim-normal"` and nothing writes it back.**
  Left alone deliberately.
  `detectMode` (`lib/mode.ts`) is what resolves the stale mode, by letting a focused textarea outrank it (issue #489).
  Changing what `exitVisual` writes would be a behaviour change wearing a refactor's clothes.

## Shortcuts

Catalog: **`crates/outl-shortcuts`**, the single source of truth, also consumed by the TUI.
The desktop fetches it via `list_shortcut_bindings` on boot, then **resolves every keystroke in TypeScript** against that cached catalog, in `installShortcuts` (`lib/shortcuts.ts`), into the handler map `lib/action-handlers.ts` builds.
No Rust resolver sits on this path: `commands/shortcuts.rs` ships every binding and states that mode filtering is the frontend's job, since the active mode shifts as the user clicks into a textarea or opens the picker.
**A mode-specific match wins over Global, and that precedence is load-bearing.**
`Cmd+Shift+X` is `WrapStrike` in Insert and `RunCodeBlock` in Global, and `defaults.rs` lists the Global chrome rows first for help-overlay readability.
So a single-pass match fires `RunCodeBlock` inside a textarea, executing the fence instead of striking the user's selection; `cmd_shift_x_splits_between_insert_and_global` (`outl-shortcuts/src/lib.rs`) pins the split.
Chord tables are [`docs/shortcuts.md`](shortcuts.md); which client performs which action is `outl_shortcuts::support`, rendered in [`docs/client-parity.md`](client-parity.md).

Two chords also have **visible icon affordances**, and quick capture is its own surface: [`docs/desktop-ux.md`](desktop-ux.md#chrome-toggle-bar-and-quick-capture).
An affordance carries no business logic — clicking flips the same store signal the dispatcher flips, so button and keyboard cannot drift.

### Keyboard reach for the chrome — `lib/keyboard-reach.ts`

Everything the mouse can hit, the keyboard reaches without a chord to memorise.
`rovingKeydown` is the roving-focus handler every toolbar and panel list wires as `onKeyDown` (Sidebar, ChromeToggleBar, SettingsModal, and the Reminders / Tasks / Timeline / Trash / Marketplace panels).
`cycleRegion` is what `F6` / `Shift+F6` walk, over the elements marked `data-outl-region`.
These are chrome navigation, not catalog actions, so they are **not** in `outl-shortcuts`: a binding there is one every client consumes, and only this client has regions.

Three rules a contributor will trip on, each bought by a regression:

- **`items` is the container saying what its stops are, and that question is only the container's.**
  `FOCUSABLE` is a selector *list*, so a row holding a button is two stops and arrowing off row 0 lands on row 0's own button.
  `tabindex="-1"` removes nothing, because `button` still matches on its tag.
- **Focus is resolved *up* to its item before the index is taken** (`activeItem`), and not with `closest`.
  Clicking a button focuses it on WebKitGTK, and a mutation that then refuses leaves focus there, outside the narrowed stops.
  `indexOf` answered `-1`, `(-1 + 1) % len` is `0`, so ArrowDown jumped to row 0 and ArrowUp to row n-2.
  `closest` does not cover it either: `TrashPanel`'s Restore button is the row's *sibling*, so the walk has to accept an ancestor that **is, or holds,** an item.
  Focus genuinely outside every item keeps `-1`, which is what lets an arrow from the container still enter at the top.
- **`focusFirstRowOnOpen` fires once per open, never per fetch, and never while loading.**
  Every panel mutation refetches, so a rule keyed on the data throws the user back to the top each time; and on a reopen the resource still holds the previous open's rows, so focus landed on one of those is lost when the fresh fetch replaces them.

`followSelection` is the fourth export and answers a different question: it keeps the selected block on screen (`block: "nearest"`, so it is inert while the row is visible).
It is keyed on the selected id rather than wired into the two movement handlers, so `g g`, `G`, a search hit and a jump out of a panel all inherit it.

### OS-standard chrome and undo / redo

Desktop-specific: `Cmd/Ctrl+Shift+X` runs the focused / selected code block, since plain `Cmd/Ctrl+X` is OS cut / view-mode block cut.

### Undo / redo (Normal mode — fire when no textarea is focused)

Deliberately **Normal**, not Global: with a textarea focused the chord falls through to the webview (the in-flight draft is the textarea's own undo domain), and a Global binding would `preventDefault` it away.
History is **block-level**: every mutation that goes through `finish_in_page` pushes the page's pre-mutation `.md` render onto a bounded per-page stack (`outl_actions::history::HistoryStacks`).
Undo restores that snapshot through `outl_md::reconcile_md`, so the restore is itself ops in the log — nothing is rewritten.
Fold toggles bypass `finish_in_page` and are not undoable, matching their "view state, not content" semantics.
Invalidation is **surgical**: a workspace **switch** clears every stack, but a peer-driven **reload** drops only the stacks of pages whose projection actually changed across it.
Restoring one of those would silently revert the peer's edits; pages the peer did not touch keep their full depth.

### Inline markdown (Insert mode — fire when a textarea is focused)

`lib/markdown-wrap.ts` owns these: each handler reads `document.activeElement`, splices the value, dispatches an `input` event so the Solid signal stays in sync, then repositions the caret / selection.

### Paste (with and without formatting)

Behaviour + routing: [`docs/paste.md`](paste.md).
Three guards, which mobile mirrors.
A code-fence host bails `Cmd/Ctrl+V` to the native splice (`detectFence` early-return) so it stays literal.
`Cmd/Ctrl+Shift+V` reads through `tauri-plugin-clipboard-manager` to dodge the macOS webview "Paste" gate.
And both pass `textarea.value`, so the draft commits first.

`components/outline-paste.ts` (`createPasteOps(deps)`) owns the three steps between the route decision and the workspace: flush the draft, run the command, fold the reply back in.
Its own module, not two closures in `<OutlineView />`, like mobile's `Journal.block-ops.ts` — so the sequence has `outline-paste.test.ts`, which no component test replaces.

**The third step is not optional.**
Applying the view alone left the editor on the pre-paste string, which `commit()` wrote straight back on the next Esc or blur.
That lost the paste from the block *and* the `.md`, with nothing on screen ([#488](https://github.com/outlmd/outl/issues/488)).
`@outl/shared/paste`'s `pasteEditorUpdate` owns that decision.
`<BlockEditor />`'s `applyPaste` writes `ta.value` as well as calling `setText`, because the caret has to move in the same frame.
Solid's `value` binding lands in a later effect, and `setSelectionRange` against the stale value clamps to the old length.
Regression net: `outline-paste.test.ts` plus `BlockRow.test.tsx` → "BlockRow paste into the block being edited — #488".

`create_block`: stale `after_id` (`NotInTree`) → append at page end (fixes `o`-key crash after peer reload).

### Block-editor chords (inside a block's textarea)

Load-bearing notes a contributor needs:

- **Plain `Enter` → splits the block at the caret** (`onEnter`, issue #184).
  `Shift+Enter` → literal `\n` soft break (issue #119), handled in `BlockEditor`'s `handleKeydown` (not the catalog; see the code comment).
- **A bare arrow key at the block's edge walks into the neighbour without leaving Insert.**
  The outline reads as one continuous document, which the TUI has shipped all along; `components/outline-caret-nav.ts` is this client's half.
  `@outl/shared/caret-nav` owns the whole decision, so the three clients cannot disagree about when a keystroke leaves a block or where the caret lands in the next one.
  Three things here are load-bearing, and the editor's own comments carry the mechanism:
  **the target is resolved before anything is awaited** (a `preventDefault` only counts during the synchronous dispatch, so a handler that commits first and looks for a neighbour after has already let the browser move the caret);
  **nothing is committed when the keystroke goes nowhere**, because `commit()` writes `editingBlockId = null`, so a `←` in the page's first block would otherwise drop the user out of Insert instead of doing nothing;
  and **`stopImmediatePropagation` is what stops a double move** — `↑` / `↓` are the catalog's Normal `SelectionUp` / `SelectionDown`, and the commit unmounts the textarea mid-dispatch, so the `window` listener would resolve the same keystroke a second time.
  Removing that line moves the caret two blocks and leaves the cursor and the editor disagreeing about where it went; `BlockRow.caret-nav.test.tsx` is the net, verified to fail without it, and it also pins that a modified arrow (`Alt+↑` moves the block) never walks.
  **A held arrow's repeat is swallowed while a step's commit is in flight** (`caretStepping`): a changed draft keeps the textarea mounted for a backend round-trip, and every repeat in that window would otherwise commit the same draft again.
  The walk is `visibleRoots()`, so a zoom is honoured, and it stops at the outline's edges rather than stepping into the backlinks below —
  those rows are editable now, so walking into one would open *another page* for editing from a keystroke aimed at this one.
- **`Tab` / `Shift+Tab` indent / outdent the block and keep it in edit mode, on itself** (issue #477).
  `commit()` (called first, so the backend sees the latest draft) flips `editingBlockId` to `null`, and the moved block's row remounts a fresh `<BlockEditor />` elsewhere in the tree.
  `onIndent` / `onOutdent` (`components/outline-structure.ts`'s `createStructureOps`) re-select and re-edit the block's own id once the move settles.
  That holds whether the move landed, was a backend no-op (`indent_block` / `outdent_block` swallow `NoPreviousSibling` / `AlreadyAtRoot`), or the command rejected outright.
  The caret survives the remount through `appState.caretIntent`, widened from the `"end"` / `"start"` sentinels to also carry a codepoint offset.
  The Tab handler captures it before `commit()` runs; the fresh `<BlockEditor />` applies it on mount and clears it, the same contract `EnterInsertAtEnd` already had.
  **The handler calls `stopImmediatePropagation`, and removing it moves the block twice** — the editor unmounts itself mid-dispatch, so `window` then resolves the same keystroke as the catalog's Normal row.
  The handler's comment carries the mechanism; `BlockRow.tab-dispatch.test.tsx` is the net, in both directions.
- `Cmd/Ctrl+X` (cut) and `Cmd/Ctrl+Z` (undo) deliberately fall through to the webview — no catalog binding matches inside a textarea.
  Native per-keystroke undo is still broken: the controlled `value={draft()}` binding resets the textarea's undo stack on every keystroke (issue #80).
- Bracket auto-pairing lives in `@outl/shared/autocomplete` (`autoPairBracket` / `autoDeletePair`, TUI parity), never here.
- The four inline autocomplete popups share one keyboard contract via `handlePopupNav` (`lib/popup-nav.ts`, unit-tested): arrows cycle, `Enter`/`Tab` with no modifiers accept, `Esc` closes.
  `Shift+Tab` outdents on every popup now (the page-ref one used to accept it).
- What each popup is triggered by and inserts: [`docs/desktop-ux.md`](desktop-ux.md#inline-autocomplete-popups).
  How a clicked `[label](url)` or `assets/…` link is routed (scheme guard, OS default app, the two import paths): [same page](desktop-ux.md#opening-an-external-labelurl-link).
- **The `/` menu lists this client's own actions, plus `/upload`, the workspace's templates and every plugin command.**
  Without the native half it was only those last three, so a workspace with no templates and no plugins opened a **one-row** popup while the TUI's `/` listed 31 built-ins.
  The rows come from `paletteActions` (`lib/palette-results.ts`) — the `Cmd+K` palette's own list, so "what can this client run" is answered once.
  **The name on each row is `outl_shortcuts::command_name`, read off `ActionSupportDto.command`, never derived here.**
  This client kebab-cased the variant (`/open-today`) while the TUI carried its own literal (`today`); two of 66 names coincided.
  An action the catalog did not name is **dropped** rather than named locally — the variant is not typeable after a `/`, so a fallback row would be visible and unreachable.
  **A native pick runs through `runNativeSlashCommand` (`lib/shortcuts.ts`), never a dispatcher of its own**, so a command cannot mean one thing from a chord and another from the menu.
  `installShortcuts` publishes that list and drops it on teardown, for the reason `loadPluginBindings` is not module-cached: a workspace swap brings a new handler map.
  Only the plugin / template half is cached in `block-suggest.ts` — an empty native list cached before the dispatcher installed would leave the menu short forever.
  Net: `BlockRow.slash-menu.test.tsx`, a real `<BlockRow />` against a real dispatcher.

### Vim parity (Normal + Visual)

Chord list: [`docs/shortcuts.md`](shortcuts.md) — don't duplicate it here.
This section captures only the **architectural decisions** a contributor needs before touching the handler modules.

- **Three categories of vim op**, by what each needs from the cursor model.
  **Block-level** ops work on `selectedBlockId` or a range of ids, and are implemented.
  **Char-cursor** ops need a character cursor inside the block, which the desktop does not have (only an id).
  **Pending-input** ops read a second character first, which the dispatcher cannot do, so they count as char-cursor — blocked either way.
  Which chord sits in which bucket is `outl_shortcuts::support`, never a list here.

- **The 10 char-cursor entries have no handler at all.**
  `support` marks them `Missing(why::NO_CHAR_CURSOR)` and the dispatcher shows the catalog's sentence, so the message can't drift (invariant 12).
  Their catalog entries stay, so the help overlay still lists them.

- **Visual mode is real, and reachable without vim (issue #23).**
  `Mode::"vim-visual"` in the store with `visualAnchorId` + `lastVisualRange`.
  `SelectRange{Down,Up}` is bound in **both** Normal and Visual and grows the range via the DOM "nothing focused" → Normal fallback, so one machinery serves vim and non-vim.
  `extendVisualRange` stays in the outline, never crossing into the backlinks — a range op would land on another page's blocks.
  Every exit funnels through one `exitVisual()`, which captures `lastVisualRange` for `g v`.
  Nothing writes `mode` back afterwards, so `detectMode` lets a focused textarea or input outrank a stale resting `vim-normal` (issue #489); `vim-visual` and open overlays stay ahead of focus.

- **`<BatchToolbar />`** (`src/components/BatchToolbar.tsx`) floats while `mode === "vim-visual"` and fires the **same** `handlers` the keyboard does, so button and chord can't drift.
  Its **Delete** confirms when a selected block has children; keyboard `d` does not (vim convention).

- **The block clipboard is Normal-mode only**, and two neighbours are tried-and-rejected rather than merely true.
  `O` creates with `beforeId` → `create_before`; **never reintroduce** the old create-at-tail plus `moveBlockDown` loop.
  The Visual highlight is a memoised `visualSet` (`Set<id>`) at the parent, because a per-row predicate is O(N²) per keystroke.
  `isInVisualRange` survives in `@outl/shared/outline` for tests only, and mobile's `Journal.tsx` mirrors `visualSet` by that name.
  The rest are rules too — range-op walk order, the fold walks, `A`'s caret intent, the clipboard's tagged value — each with an obvious alternative that is wrong quietly.
  They live in [`docs/shortcuts.md` → Desktop internals](shortcuts.md#desktop-internals-the-range-and-fold-handlers); read one before changing it.

### `Enter` outside a textarea (Normal mode)

With a block selected and nothing focused (the DOM fallback puts the dispatcher in Normal even with `vim_mode == false`), `Enter` resolves to the shared `OpenRefUnderCursor` —
and the desktop handler **always enters Insert on the block under the cursor, with no exceptions**.
A **backlink row** used to be the one carve-out: `Enter` opened the source page, because the row was read-only.
It is not read-only any more, so the exception went with the thing that justified it, and both chords that can reach a row (`i` and `Enter`) go through the single `onBacklink.enterInsert()`.
That is also what the TUI answers, where `Focus::Backlink` means `enter_insert_backlink` — so the two clients agree on `Enter` for the same reason rather than by coincidence.
The source page stays reachable without a chord: the group header is a `<button>`, which `lib/keyboard-reach.ts` makes a keyboard stop.

Why diverge from the TUI on the *ref*: it has a char cursor, so "open the ref under cursor" is well-defined there; the desktop only has a selected block, so **following a ref is the click on the token** (`onRefClick`) and `Enter` means edit.
