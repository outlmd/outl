# CLAUDE.md — outl-desktop

Tauri 2 desktop client (macOS, Linux, Windows).
Solid + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation delegates to `outl-actions`.

Feature-by-feature behaviour — what a surface shows, what a popup inserts, what a panel lists — is [`docs/desktop-ux.md`](../../docs/desktop-ux.md).
This file keeps the rules: why something is the way it is, and the regression each guard bought.

## Status

**Feature-complete v0**, with `desktop.yml` CI, signed bundles and the Homebrew cask on top.
Per-feature state is [`docs/client-parity.md`](../../docs/client-parity.md), which is generated.

## Layering

```text
outl-core      (CRDT, op log, storage trait)
outl-md        (.md parse/render, sidecar)
outl-actions   (workspace operations + SyncEngine, shared with TUI + mobile)
   ↑
outl-desktop   src-tauri/ (commands: parse args → outl-actions → render)
               src/       (Solid + Tailwind + @outl/shared)
```

## Hard rule

**This crate adds no business logic.**
Anything involving the workspace shape (edit, move, todo, journal render) delegates to `outl-actions` — root `CLAUDE.md` → "Shared logic" is the owner of that rule.
Same rule on the frontend: before writing a helper under `src/lib/`, check `@outl/shared` ([`crates/outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md)).
It owns the inline-token renderer, paste detection, ref autocomplete, the DTO types and the shared command wrappers.

**Do not hand-write a `#[tauri::command]` wrapper.**
Every purely-delegating command module here is a single macro invocation — `outl_tauri_shared::block_commands!(crate::state::AppState);` and friends — declared once in that crate's `wrappers/catalog.rs` so both clients get it.
Which stay hand-written (those needing more than `State<'_, AppState>`), why a client takes a whole module, and the parity test: [`outl-tauri-shared/CLAUDE.md`](../outl-tauri-shared/CLAUDE.md) → "One command surface, not two".

What this crate **does** own: path discovery, the desktop-only command surface, the Solid 3-pane frontend, and an `AppState` holding a `ProjectionWriter` for async `.md` + sidecar writes.
Plus the cross-platform `notify` FS watcher that signals the frontend when peer `ops-*.jsonl` files grow — what replaces the `NSMetadataQuery` dance iOS forces on mobile.
`run_code_block` is a **thin adapter**: the orchestration is `outl_actions::exec`, shared with mobile, so adding behaviour to `commands/exec.rs` is almost always a smell.

## Layout

The frontend / `src-tauri` file tree (which component owns what, where each `lib/*.ts` helper lives) is in [`docs/development.md`](../../docs/development.md#desktop-crate-layout).

### `action-handlers.ts` is eight files

It was 1,038 lines, and it sat at its recorded size baseline, so nothing could be added to it.
It is the **assembly** now: build the context, spread the groups, carry the "deliberately absent" note.

The split axis is **what a keystroke changes**, not what kind of code the handler is.
A new handler goes in the sibling that owns its answer to that question, never back in the parent.
The siblings and the one-line responsibility of each are in the [layout tree](../../docs/development.md#desktop-crate-layout); each file's header doc carries the reasoning for its own boundary.

Three things the split decided on purpose:

- **Only `buildHandlers` is exported from the parent.**
  `AppShell` and every test that reaches the map import it and nothing else, so there were no import paths to update.
- **The split moved declarations, and a pass after it deduplicated the bodies.**
  Two passes on purpose: a declaration move is reviewable on sight, a body change is not.
  Same net for both: `shortcuts.support.test.ts` pins the **presence** of all 66 keys in both directions, and nothing about what one does.
  That is why `clipboard-handlers.test.ts` exists for the two it never reached, `YankCurrentBlock` and `YankRange`.
  The dedup collapsed eight inline copies of `ctx.runOn`, three copies of one batch loop (`ctx.runEach` owns it now), the `Esc` cascade's second list of panels, and seven toggles written two ways.
- **`exitVisual` still writes `mode: "vim-normal"` and nothing writes it back.**
  Left alone deliberately.
  `detectMode` (`lib/mode.ts`) is what resolves the stale mode, by letting a focused textarea outrank it (issue #489).
  Changing what `exitVisual` writes would be a behaviour change wearing a refactor's clothes.

### `BlockRow.tsx` is five files

It was 1,425 lines.
It is the row **chrome** now: selection / Visual / pending-cut / drop-target state, indent guides, fold chevron, the bullet-vs-checkbox gesture split, the [quote wrapper](../../docs/desktop-ux.md#blockquote-chrome), the recursion.
**New code of one of these shapes goes in the sibling, not back in the parent** — the same axis `action-handlers.ts` splits on.
The siblings are in the [layout tree](../../docs/development.md#desktop-crate-layout), and each one's header doc carries the reasoning for its own boundary.
Five of those boundaries rest on a rule, not a preference.
`BlockBody.tsx`'s read-mode branches are flat and mutually exclusive because a table has no cell editor and a fence has no fence editor, by design ([#329](https://github.com/outlmd/outl/issues/329)).
`BlockEditor.tsx` is mounted only while editing, which is why focus and pending caret intent are `onMount` and not an effect on an `editing` flag.
`block-suggest.ts` is one module because **only one popup is ever open at a time** is the invariant all of it rests on; detection and span replacement stay in `@outl/shared/autocomplete`.
`CodeFenceView.tsx`'s `RichFenceFrame` iframe is sandboxed `allow-scripts`, never `allow-same-origin`.
And `block-callbacks.ts` holds the `BlockCallbacks` interface alone, re-exported from `BlockRow.tsx` so `import { type BlockCallbacks } from "./BlockRow"` keeps resolving.

Two things the split decided on purpose:

- **The draft signal stays on `<BlockRow />`.**
  Its lifetime is the row, not the edit session.
  Moving it into `<BlockEditor />` would re-seed it from `props.block` on every entry into Insert, which is a behaviour change (it would mask the bug below), not a move.
- **`startEdit` no longer calls `focusTextarea()`.**
  The editor focuses itself on mount, in the microtask the removed call landed in anyway.

**Known, pre-existing, not fixed here:** the draft is seeded once per row instance, and only `startEdit` re-seeds it.
`setOutline` reconciles on id, so a block a peer rewrote keeps its row instance and its old draft.
Entering Insert by a path that bypasses `startEdit` (`i` / `A`, which write `editingBlockId` straight from the handler modules) then shows the stale text and commits it over the peer's.

## First-run onboarding

`components/Onboarding.tsx` is the flow and `App.tsx` gates between it and `<AppShell />`; the two steps are in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#first-run).

The "has the user onboarded" flag is a **per-install UI flag in `localStorage`** (`outl.onboarded`), **not** workspace state — it deliberately does NOT go through the op log.
It is intentionally not in `settings.json` either, since `settings.last_workspace` is the only first-run signal the backend tracks.
The **copy** lives in `@outl/shared/onboarding`, identical to mobile, and pairing is **not** reimplemented — `Onboarding` renders the real `<SyncPanel />`.

## Backlinks, nested pages, and the loading rule

`components/PageSections.tsx` mounts `<InlineBacklinks />` then `<NestedPages />` below the outline (issue #275); what each renders is in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#backlinks-and-nested-pages-below-the-outline).
They look alike and answer different questions — backlinks point *at* this page, nested pages live *under* its namespace — which is why the second is **not** gated on `backlinksOpen`.
Folding the toggles together would hide one when the user muted the other.

**Page deletion has two doors and one flow.**
`g d` routes through `DeletePage`, which runs the same `window.confirm` + `deletePage(slug)` path as the sidebar row's hover `×` ([gesture](../../docs/desktop-ux.md#sidebar--hover--to-delete-a-page)).
Journals are refused, and the body is the shared `outl_tauri_shared::commands::page::delete_page` — no desktop logic on either door.

`OutlineView.tsx` refetches backlinks via `createEffect(on(slug, …))` — per navigation, not per commit (the "Esc is slow" fix).
All three fetch sites (that effect, `AppShell::refreshActivePage`, `toggleBacklinkTodo`) go through `trackBacklinks` in `lib/store.ts`, none of them awaited on a render or keystroke path.
It owns `appState.backlinksLoading`, which `<InlineBacklinks />` is the **only** reader of.
The scan is `O(blocks-in-workspace)`, so the section carries its own `loading…` mark and placeholder rows while the outline keeps the caret.
A page-wide load state here is the regression [`UX.md` → Never block a keystroke](../../UX.md#2-never-block-a-keystroke) now names; `InlineBacklinks.test.tsx` → "backlinks load behind the page" is the net.
A refetch keeps the rows it already has — blanking them reads as "this page lost its references" — so only a first load shows the placeholder.

**Toggling a task from a backlink row (issue #144)** mutates the *source* page, and the backlink effect is keyed on the slug, which didn't change.
So `toggleBacklinkTodo` refetches `pageBacklinks(currentSlug)` itself — the exception that effect's comment calls out.
Never refresh it by re-opening via `openRef`: `PageView.backlinks` is always empty now, so that blanks the section.

## Properties (`key:: value`) — `PropertyEditor.tsx`

`components/PropertyEditor.tsx` replaced `@outl/shared`'s `<BlockProperties />` here, which edits an existing chip's value and nothing else.
Creating the first property therefore meant the TUI or the `.md`, and deleting was an invisible gesture (issue #13).
The three verbs it owns are in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#properties-key-value-chip-row).

The chord has no chip to click, so `AddProperty` writes `appState.addPropertyBlockId` and `<BlockRow />` opens a blank editor on that id.
`OpenProperties` (the TUI's `g p`) resolves to the same handler: that overlay exists to *list* a block's properties, and the chip row is already that list permanently, so the only half it lacks is the blank pair.
The editor calls `onAddOpenChange(false)` on close, which is what lets a second press re-open it on the same block.
The page-property row is hidden while zoomed — the header is a *block* then, and page metadata under a block title reads as the block's.

`propertyChips` stays in `@outl/shared/markdown` — wrapped, never reimplemented.
Promoting `PropertyEditor` itself waits on mobile: its answer to the same issue is a sheet with tappable key chips, not an inline row, so promoting a desktop-shaped API first would freeze the wrong contract.

## Page history — `TimelinePanel.tsx`

The panel (issue #241) and its two restore rows are in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#page-history-panel); `appState.timelineOpen` is the flag.

Three things a contributor should not undo:

- **It is not the undo stack.**
  `Cmd+Z` walks this session's mutations (`outl_actions::history`); this walks every device's, from the workspace's start.
  The panel and the command are named `timeline` precisely so the two don't get merged.
- **A restore is new ops, never a rewind**, and `@outl/shared/trust`'s `restorePlan` is the single owner of which rows carry one.
  It refuses the cases where "the version before" is a position or a value whose meaning depends on its neighbours.
- **Nothing here decides what an event is.**
  `outl_actions::timeline` owns which blocks count as the page's, which ops are not events, and the order.
  `formatAt` renders `at_ms` and must never *re-sort* on it — the ordering is HLC with actor tiebreak and already applied, so a device with a skewed clock can legitimately show a time that reads out of order.

The fetch is gated on `timelineOpen && appState.page`: the read walks every block in the page under the workspace lock, which is not something to do on every navigation on the chance the user might open the panel.
No chord yet — a global binding means an entry in `outl-shortcuts`, which every client consumes, and the TUI has no timeline surface to bind it to.

## "This page isn't syncing" banner

`<PageAheadOfLogBanner client="desktop" />` (`@outl/shared/warnings`) renders above the outline when `PageView.md_ahead_of_log` comes back set.
`aheadOfLogNotice` owns the copy; `client="desktop"` is what makes it name `outl reconcile --ahead-of-log`.
What the condition means and why it has to reach the user: [`docs/clients.md` → Surfacing a page that stopped syncing](../../docs/clients.md#surfacing-a-page-that-stopped-syncing).

`appState.mdAheadOfLog` is sticky per page (`stickyAheadOfLog`), because only the open commands carry the flag — a mutation reply would otherwise clear the banner on the user's first edit, the very action it warns against.
Sticky stops at `PageView.md_ahead_of_log_checked`: a reply that ran the check is authoritative both ways, so the next open after a reconcile clears the banner instead of leaving a healthy page marked broken.

## Theme tokens

`applyPaletteToRoot` (`@outl/shared/theme`) writes only the canonical `--color-outl-*` namespace, and `src/styles.css`'s `@theme` block declares only that one.
The legacy `--color-ios-*` / `--color-iosd-*` set is gone, which is what stops the OS appearance setting from changing markdown block elevation through Tailwind's `dark:` variant (invariant 13, RFC 0022).
Details: [`docs/theming.md`](../../docs/theming.md#desktop-css-custom-property-namespaces).
`commands/theme.rs` is a thin wrapper over `outl_tauri_shared::commands::theme`, so mobile registers the identical two commands rather than hardcoding palette values.

## Running and tests

Dev + bundle commands, why Vite runs on port 1421, and what a terminal-launched dev build logs: [`docs/development.md`](../../docs/development.md#desktop-tauri-2).
Test layers, tooling and the frontend suites: [`docs/development.md`](../../docs/development.md#per-client-test-suites).

## Shortcuts

Catalog: **`crates/outl-shortcuts`**, the single source of truth, also consumed by the TUI.
The desktop fetches it via `list_shortcut_bindings` on boot, then **resolves every keystroke in TypeScript** against that cached catalog, in `installShortcuts` (`lib/shortcuts.ts`), into the handler map `lib/action-handlers.ts` builds.
No Rust resolver sits on this path: `commands/shortcuts.rs` ships every binding and states that mode filtering is the frontend's job, since the active mode shifts as the user clicks into a textarea or opens the picker.
**A mode-specific match wins over Global, and that precedence is load-bearing.**
`Cmd+Shift+X` is `WrapStrike` in Insert and `RunCodeBlock` in Global, and `defaults.rs` lists the Global chrome rows first for help-overlay readability.
So a single-pass match fires `RunCodeBlock` inside a textarea, executing the fence instead of striking the user's selection; `cmd_shift_x_splits_between_insert_and_global` (`outl-shortcuts/src/lib.rs`) pins the split.
Chord tables are [`docs/shortcuts.md`](../../docs/shortcuts.md); which client performs which action is `outl_shortcuts::support`, rendered in [`docs/client-parity.md`](../../docs/client-parity.md).

Two chords also have **visible icon affordances**, and quick capture is its own surface: [`docs/desktop-ux.md`](../../docs/desktop-ux.md#chrome-toggle-bar-and-quick-capture).
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

Behaviour + routing: [`docs/paste.md`](../../docs/paste.md).
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
- `Cmd/Ctrl+X` (cut) and `Cmd/Ctrl+Z` (undo) deliberately fall through to the webview — no catalog binding matches inside a textarea.
  Native per-keystroke undo is still broken: the controlled `value={draft()}` binding resets the textarea's undo stack on every keystroke (issue #80).
- Bracket auto-pairing lives in `@outl/shared/autocomplete` (`autoPairBracket` / `autoDeletePair`, TUI parity), never here.
- The four inline autocomplete popups share one keyboard contract via `handlePopupNav` (`lib/popup-nav.ts`, unit-tested): arrows cycle, `Enter`/`Tab` with no modifiers accept, `Esc` closes.
  `Shift+Tab` outdents on every popup now (the page-ref one used to accept it).
- What each popup is triggered by and inserts: [`docs/desktop-ux.md`](../../docs/desktop-ux.md#inline-autocomplete-popups).
  How a clicked `[label](url)` or `assets/…` link is routed (scheme guard, OS default app, the two import paths): [same page](../../docs/desktop-ux.md#opening-an-external-labelurl-link).

### Vim parity (Normal + Visual)

Chord list: [`docs/shortcuts.md`](../../docs/shortcuts.md) — don't duplicate it here.
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
  `extendVisualRange` stays in the outline, never crossing into read-only backlinks.
  Every exit funnels through one `exitVisual()`, which captures `lastVisualRange` for `g v`.
  Nothing writes `mode` back afterwards, so `detectMode` lets a focused textarea or input outrank a stale resting `vim-normal` (issue #489); `vim-visual` and open overlays stay ahead of focus.

- **`<BatchToolbar />`** (`src/components/BatchToolbar.tsx`) floats while `mode === "vim-visual"` and fires the **same** `handlers` the keyboard does, so button and chord can't drift.
  Its **Delete** confirms when a selected block has children; keyboard `d` does not (vim convention).

- **The block clipboard is Normal-mode only**, and two neighbours are tried-and-rejected rather than merely true.
  `O` creates with `beforeId` → `create_before`; **never reintroduce** the old create-at-tail plus `moveBlockDown` loop.
  The Visual highlight is a memoised `visualSet` (`Set<id>`) at the parent, because a per-row predicate is O(N²) per keystroke.
  `isInVisualRange` survives in `@outl/shared/outline` for tests only, and mobile's `Journal.tsx` mirrors `visualSet` by that name.
  The rest are rules too — range-op walk order, the fold walks, `A`'s caret intent, the clipboard's tagged value — each with an obvious alternative that is wrong quietly.
  They live in [`docs/shortcuts.md` → Desktop internals](../../docs/shortcuts.md#desktop-internals-the-range-and-fold-handlers); read one before changing it.

### `Enter` outside a textarea (Normal mode)

With a block selected and nothing focused (the DOM fallback puts the dispatcher in Normal even with `vim_mode == false`), `Enter` resolves to the shared `OpenRefUnderCursor` — but the desktop handler **always enters Insert on the selected block**.
The one exception: on a **backlink row** (read-only), `Enter` opens the source page and lands the cursor on the referencing block.

Why diverge from the TUI: it has a char cursor, so "open the ref under cursor" is well-defined there; the desktop only has a selected block, so **following a ref is the click on the token** (`onRefClick`) and `Enter` means edit.

## Zoom / focus into a block

The chrome (page-like header, zoom path, which blocks render) is in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#zoom--focus-on-a-block), the cross-client contract in [`docs/clients.md`](../../docs/clients.md#zoom--focus-on-a-block-roamworkflowy).

Three decisions beyond that contract:

- **No zoom stack:** `ZoomOut` reads the current focus's breadcrumb — last crumb is the parent, empty breadcrumb means top-level so it exits to the full page.
- **Bullet gesture split**, so zoom cannot collide with task state: a `•` bullet zooms via `onFocusBlock`, a `▢` / `▨` / `▣` checkbox keeps its one-stop-per-click toggle, and the fold **chevron** stays collapse.
  DOING shares TODO's colour and is told apart by the half-filled glyph; only DONE strikes the body.
- **`j`/`k` stay inside the zoom:** `SelectionUp`/`SelectionDown` walk `navBlocks()` (`[fv.root]` when zoomed) so the cursor can't escape the subtree.

## Reminders (`remind::`)

`<RemindersPanel />` lists them, `set_block_remind` authors a rule, `<AppShell />` polls `deliver_due_reminders` every 30s.
Panel wiring, the chord choices and the `[reminders] enabled` default: [`docs/reminders.md`](../../docs/reminders.md#desktop-outl-desktop).
The one rule here: **nothing about when a reminder fires is computed in the frontend** — the instants come from `outl_actions::reminders`, the labels from `@outl/shared`.

## Task list — `TasksPanel.tsx`

What the panel lists and how it opens: [`docs/desktop-ux.md`](../../docs/desktop-ux.md#task-list-panel).
No longer mobile-only: `Capability::TaskList` is `Full` here, and only the TUI keeps the `query`-block nudge.
Order and grouping are the backend's — `outl_actions::tasks` decides, `groupTasksByPage` sections, and nothing here ranks.

## Trust and polish

- The saved mark: `savedMarks` in `lib/store.ts`, written when `edit_block` resolves and rendered by `BlockRow` as a fading dot.
  Do not mark before the command resolves — the mark means "the op is on disk".
- Focus mode is **not an `Action` yet**: `support.rs` and `defaults.rs` sit at their file-size baseline, so a chord waits for that split.
- Motion, focus mode and the first-run tips come from `@outl/shared/polish`: [`docs/desktop-ux.md`](../../docs/desktop-ux.md#motion-focus-mode-first-run-tips).

## Plugins

The effect layer, the toolbar and the view surfaces: [`docs/plugins.md`](../../docs/plugins.md) → "Desktop plugin surface".

## Logging

`run()` in `src-tauri/src/lib.rs` ignores `SIGPIPE`, probes the Windows webview runtime, calls `outl_telemetry::init`, then installs a `tracing_subscriber` fmt subscriber.

**The Windows probe comes before `init` on purpose**, so the crash reporter is not running on the path that exists to avoid a report.
`src-tauri/src/webview_boot.rs` owns both Windows boot failures: a missing WebView2 runtime, and a present runtime that times out handing Tauri a controller (OUTL-4).
Those, the sentences the user gets, and the rules that are easy to undo: [`docs/development.md`](../../docs/development.md#why-the-desktop-crate-has-windows-only-boot-code).
Both decisions are taken against traits, not `tauri::AppHandle` — the only reason either is testable off Windows, so do not inline them back.

The subscriber writes through `outl_telemetry::lossy_stderr` with `log_internal_errors(false)`.
A closed stderr — a desktop launched with no terminal — must not panic, and `eprintln!` turns `EPIPE` into `failed printing to stderr`, including in the fmt layer's own error report.
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`; what a terminal-launched dev build shows is in [`docs/development.md`](../../docs/development.md#desktop-tauri-2).
Init uses `set_global_default`, not `.try_init()`: `try_init` also claims the `log` slot, which `tauri-plugin-log` needs.

## Settings

Stored through `outl-config` at the shared global config path; the flat JSON DTO `settings.rs::Settings` hands the frontend, field by field, is in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#settings-modal).

`settings.rs::restore_unmodeled_sections` lists only fields the modal does not own.
**A new unmodeled config field omitted there will get silently dropped on the next modal save.**
`backlinks_order` is the same pattern from the other side: it rides the DTO but `save` restores it from disk, so the modal cannot clobber the dedicated `set_backlinks_order` command's write.

### Privacy: the crash-reporting toggle writes outside Save

`SettingsModal`'s **Privacy** section (RFC 0377) is the one row that does not live in `draft()`.
`[telemetry] enabled` is not in the flat `Settings`, and `restore_unmodeled_sections` puts the section back from disk on every save.
So routing it through Save would make the toggle inert, and dropping the restore would flip a user's opt-out back on with every unrelated save, because the default is `true`.
It reads `getTelemetryEnabled()` on every open and writes `setTelemetryEnabled()` on change, the same split the backlinks direction button has.
On open rather than `onMount`, because `AppShell` mounts it for the app's lifetime and another process can write the flag while the window runs (`outl telemetry off` in a terminal).
The switch moves only on the command's reply: an optimistic flip that fails to save leaves a checkbox reading "off" over a reporter that is still on.
The words are `CRASH_REPORTS_SETTING` (`@outl/shared/onboarding`), never written here.

**This section is where a user *looks*; it is not where they are *told*.**
Reporting is on by default, so a disclosure that lives only here reaches only the curious.
The proactive half is `<CrashReportsNotice />` (`@outl/shared/polish`, RFC 0377 §6), mounted at the **shell**, not above the outline, because per page it would re-run its dismissed check on every navigation.

## Peers

`commands/peers.rs` carries no business logic: `outl_sync_iroh::PeersStore` owns the store, and `peers.json`'s per-graph location is in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#paired-devices).
`outl_sync_now()` just calls `Arc<dyn SyncTransport>`'s trait method — the trigger behind Refresh ([panel](../../docs/desktop-ux.md#sync-panel)).
What that pass actually does: [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

The always-visible capsule dot ([states](../../docs/desktop-ux.md#sync-status-dot)) and the panel's derive reachability from the same `peersOnline` (`@outl/shared/peers`) mobile uses.
**Do not add a second reachability path**; `peersOnline` is the one owner.
When adding a control to the top-right capsule, keep it in the same order as mobile's top bar.

### When this window does not hold the device endpoint

A device binds **one** iroh endpoint, and a lease decides which process gets it — not being the GUI ([`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "One endpoint per identity, elected not assigned").
So the desktop can run with `iroh_transport` / `iroh_pairing` empty while sync works fine through the shared `ops/` dir; a co-resident `outl mcp serve` that started first is the ordinary case, since Claude Desktop launches it at login.

`iroh_sync::no_endpoint_reason()` separates why those slots can be empty, because the cases deserve opposite answers.
`P2pDisabled` is the user's opt-out, `HeldByAnotherProcess` lost the election, `Unavailable(why)` could not arbitrate the lease or failed to build the transport.
It is rewritten on every `wire_iroh_transport`, so a workspace swap that wins the endpoint clears the previous warning.
Only `P2pDisabled` refuses to pair; collapsing the others into it told a user whose transport failed to build to switch on a setting that was already on.

- **Pairing** falls back to the CLI's one-shot `host_pairing` / `join_pairing`, which bind their own endpoint and close it before returning.
  For those seconds it **does** take the relay route from the lease holder.
  Accepted deliberately: pairing is rare, user-initiated and short, the holder recovers when the one-shot closes, and the alternative is a user who cannot add a device at all.
  It is not a precedent — no other path in the GUI may bind an endpoint.
  With P2P simply **off** (`transport = "file"`) pairing is **refused** instead, because binding there would override the setting the user picked.
- **Refresh** returns an error naming the reason rather than an `Ok(())` that did nothing.
  The silent no-op was the actual defect: the dot stays orange, Refresh appears to work, nothing says why.
  `transport = "file"` stays a quiet no-op — the user's own choice is not a degraded state.
  It also **re-runs the election** first (`retry_endpoint`): the recorded reason is a snapshot of login, nothing here notices the holder exiting, so without that the window would explain a dead process all session.
  It re-contends only from `HeldByAnotherProcess` / `Unavailable` — `P2pDisabled` is a setting rather than a race, and a `None` reason with no transport means the boot opener has not wired yet and a second pass would race it for the same lease.

**Still missing:** the status dot cannot tell "every device is offline" from "another local process holds the endpoint".
Teaching it that state means changing `PeerStatusDto` in [`outl-tauri-shared`](../outl-tauri-shared/CLAUDE.md), not a desktop-local patch, because `peersOnline` is the one owner of reachability on both clients.
Until then the Refresh error is the only surface that explains it.

### `reload_workspace` offloads the replay off the IPC thread

The desktop command is a thin caller of `outl_tauri_shared::reload_workspace_into`.
That owns the offload, the publish-only-what-it-cannot-have-dropped retry, and why both exist: [that crate's CLAUDE.md](../outl-tauri-shared/CLAUDE.md) → "A reload publishes only what it cannot have dropped".
Only the cheap tail — history invalidation, the `Mutex` swap, the reconcile spawn — runs on the command thread here.

## "Open With → outl" (`.md` / `.txt` from the OS)

`src-tauri/src/open_with.rs` is the plumbing and all this crate adds, alongside `bundle.fileAssociations`' deliberate `role: "Viewer"` + `rank: "Alternate"`.
The namespace, the re-open rule, the refusals and the three delivery paths belong to `outl_actions::open_with` and [`docs/clients.md`](../../docs/clients.md#opening-a-file-from-the-os-open-with--outl).

Three things a contributor will trip on:

- **`RunEvent::Opened` is also where `outl://` deep links arrive on macOS.**
  `file_path_from` keeps only `file://`; without that filter a deep link would be navigated twice — once by the `deep-link` plugin, once here.
- **"Is the frontend listening" is a flag the frontend sets, never inferred, and it comes back down.**
  `setup` creates the main window inside `build()`, before the event loop starts, so *every* `RunEvent::Opened` sees a window.
  An earlier version read that as "warm" and emitted every cold-start file into a webview that had not mounted; the emit was lost and the app silently opened today's journal instead of the file the user double-clicked (issue #98).
  `take_pending_open_file` marks the flag and `AppShell` subscribes before calling it; both paths go through `route`, warm ones included, since a slow boot keeps the loading screen up long after the process is "running".
  `on_page_load` clears it again on `PageLoadEvent::Started`: a reload takes the listener with it while the process keeps running, and a latch that only goes up tells the same lie one layer in.
  `onMount` drains the open-file and deep-link buffers together — the deep link wins when both are set, but the other drains anyway, so an unread target cannot replay later.
- **`run()` is `.build(…).run(|app, event| …)` now, not `.run(generate_context!())`.**
  The event callback is the only place `RunEvent::Opened` is reachable.
  Do not collapse it back.

## Capture from outside the app (RFC 0363)

`drain_capture_inbox` plus `AppShell`'s `installForegroundRefresh` (`@outl/shared/capture`) land a line `outl capture` left in `<root>/.outl/captures/`, on mount and on every return to the foreground.
That is the whole story: outl registers **no** Service, menu bar item or global hotkey that writes one, which is why `CaptureOutsideTheApp` is `Partial`.

**The refresh is skipped while the user is in a block**, for the reason `onPeerChange` defers a peer reload: re-applying a view re-materializes the outline under the cursor.
The capture is in the op log either way, so the next refresh shows it ([`UX.md` → Never block a keystroke](../../UX.md#2-never-block-a-keystroke)).

## Deep links (`outl://`)

The scheme contract, the shared `outl_actions::parse_deep_link` parser, and this client's warm / cold wiring live in [`docs/deep-links.md`](../../docs/deep-links.md#desktop-wiring-outl-desktop).
One thing bites every time, and it applies to "Open With" too: **testing on macOS needs a bundled, installed app**.
LaunchServices only registers `outl://` and reads `CFBundleDocumentTypes` from the built bundle, so `cargo tauri dev` never sees either.

## When you're done

1. `cargo fmt && cargo clippy -p outl-desktop --all-targets -- -D warnings && cargo test -p outl-desktop`
2. `bun --filter outl-desktop test` (Vitest)
3. `cd crates/outl-desktop && cargo tauri dev` — smoke open in a real window, click the parts you touched.
4. If you touched `@outl/shared`, also run `bun --filter outl-mobile test` to confirm paridade.
5. If you changed what a surface *does*, update [`docs/desktop-ux.md`](../../docs/desktop-ux.md) in the same change.
