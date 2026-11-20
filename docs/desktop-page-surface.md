# How the desktop page surface is wired

The outline, the row, and everything drawn around and under the page: the sections below the outline, which region owns the cursor, the property editor, the history panel, the single writer of a `PageView`, the day clock, zoom, and the task panel.

What the user sees is [`docs/desktop-ux.md`](desktop-ux.md).
This is which module decides it.

Moved out of [`crates/outl-desktop/CLAUDE.md`](../crates/outl-desktop/CLAUDE.md), which is loaded into the model's context on every session in that crate.
These sections are reference: you read one when you touch that part of the client.
The rules that hold whatever you are editing stayed in `CLAUDE.md`.

## `BlockRow.tsx` is five files

It was 1,425 lines.
It is the row **chrome** now: selection / Visual / pending-cut / drop-target state, indent guides, fold chevron, the bullet-vs-checkbox gesture split, the [quote wrapper](desktop-ux.md#blockquote-chrome), the recursion.
**New code of one of these shapes goes in the sibling, not back in the parent** — the same axis `action-handlers.ts` splits on.
The siblings are in the [layout tree](development.md#desktop-crate-layout), and each one's header doc carries the reasoning for its own boundary.
`outline-structure.ts` (`createStructureOps`) re-selects and re-edits a block on itself after `indent_block` / `outdent_block`; see [Block-editor chords](desktop-keybindings.md#block-editor-chords-inside-a-blocks-textarea) ([#477](https://github.com/outlmd/outl/issues/477)).
Five of those boundaries rest on a rule, not a preference.
`BlockBody.tsx`'s read-mode branches are flat and mutually exclusive because a table has no cell editor and a fence has no fence editor, by design ([#329](https://github.com/outlmd/outl/issues/329)).
It is also where a transclusion expands: a block whose whole text is `!((blk-X))` (`embedOnlyHandle`) or `![[page]]` (`pageEmbedOnlyName`) appends the borrowed rows *below* its own content, through the shared read-only `<EmbeddedSubtree />`.
**Both expansions draw nothing until their resolve answers with rows, and that is not an optimisation.**
The resolve lands a round-trip after the first paint, in a block the user may be typing in.
A placeholder that reserves vertical space and then releases it moves every row below the caret — so this is `lateSectionVisible`'s rule ("a first load draws nothing") reduced to `count > 0`, an embed having no empty state worth keeping.
The maps come from `appState.embeds` / `appState.pageEmbeds`, both filled by `<OutlineView />`'s `resolveTransclusions` (one skip key per kind, so a newly typed `[[ref]]` does not invalidate the page-embed answer).
**Each is read through an accessor, never a `const` in the component body.**
A body runs once and is not a tracking scope, so `const x = appState.embeds[h]` captures what the map held at mount.
The resolve lands a round-trip after the page is on screen and `setOutline` reconciles on id, so the expansion showed up only when the map happened to be warm already.
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

## Backlinks, nested pages, and the loading rule

`components/PageSections.tsx` mounts `<InlineBacklinks />` then `<NestedPages />` below the outline (issue #275); what each renders is in [`docs/desktop-ux.md`](desktop-ux.md#backlinks-and-nested-pages-below-the-outline).
They look alike and answer different questions — backlinks point *at* this page, nested pages live *under* its namespace — which is why the second is **not** gated on `backlinksOpen`.
Folding the toggles together would hide one when the user muted the other.

**Page deletion has two doors and one flow.**
`g d` routes through `DeletePage`, which runs the same `window.confirm` + `deletePage(slug)` path as the sidebar row's hover `×` ([gesture](desktop-ux.md#sidebar--hover--to-delete-a-page)).
Journals are refused, and the body is the shared `outl_tauri_shared::commands::page::delete_page` — no desktop logic on either door.

**Rename and merge are the same two doors, and the same flow.**
The sidebar row's hover `✎` (rename) and `⇥` (merge into) sit beside the `×`; journals get none of the three.
Both open `PageActionDialog.tsx` (a name field prefilled with the title, or a page search that leaves the source out), because the webview has no `window.prompt`; the questions that follow are `window.confirm`, like delete.
The flow itself is `@outl/shared/pages/rename-merge` (a taken name asks before merging, a merge **always** asks, the wording is owned there), wrapped by `lib/page-actions.ts`, which only adds the confirm, the toast and the error surface.
Declining a confirm writes nothing and leaves the dialog open; the outcome (blocks moved, links updated, every property conflict) goes to `<NoticeToast />` via `appState.notice`, because a dropped property value the user never saw is the failure the report exists to prevent.
There is no chord and no menu entry: delete has none in `menu.rs` either, so these did not get one.
The sidebar list is keyed on the page id **and** slug and title, since a rename keeps the id.

**Backlinks are fetched per navigation, by `installBacklinksEffect` (`lib/backlinks-effect.ts`), and "per navigation" is `@outl/shared`'s answer, not this crate's.**
It sources `createBacklinksKey`, which is `createPageKey` with two fields — read [`outl-frontend-shared/CLAUDE.md`](../crates/outl-frontend-shared/CLAUDE.md) before changing either.

This used to be `createEffect(on(() => appState.page?.slug, …))` inline in `OutlineView.tsx`, under a comment saying it fired once per navigation.
It did not: `on()` makes a dependency explicit and **compares nothing**, and `applyPageState` writes the page as one object (`setAppState({ page, … })`), which `setState` merges key by key — so `page` is assigned a fresh `PageMeta` on every reply and its node fires.
So every created or deleted block ran `emptyBacklinks()`, `lateSectionVisible` unmounted the whole section, and it came back an `O(blocks-in-workspace)` scan later with the same rows.
A *path* write (`setAppState("page", …)`) merges in place and would not fire, which is why a store test written that way passes against the broken code — `backlinks-effect.test.ts` goes through the real `applyPageState` for that reason.
The sidebar's recents queue had the identical defect and now takes `createPageKey` directly.
`navigation/page-key.parity.test.ts` in `@outl/shared` reads both clients' sources and refuses the shape outright.

Taking the shared key also fixed a half this client never had: `namespace_children` hangs off `title::`, so a rename under the same slug now refetches.

**All five fetch sites go through `trackBacklinks` (`lib/backlinks-fetch.ts`), which takes no argument, and that is the point.**
The five are that effect, `AppShell::refreshActivePage`, `toggleBacklinkTodo`, `BacklinkRowEditor`'s text commit, and `backlinkRowOps.settle` (the nine cross-page chords).
None is awaited on a render or keystroke path.
It owns `appState.backlinksLoading`, which `<InlineBacklinks />` is the **only** reader of.
A page-wide load state here is the regression [`UX.md` → Never block a keystroke](../UX.md#2-never-block-a-keystroke) names; `InlineBacklinks.test.tsx` → "backlinks load behind the page" is the net.

Its own module and not `store.ts` for the reason `cursor-region.ts` is one: that file is the state *shape*, and this is an async policy.
The parity test's declared exception moved with it and shrank to one row, since no caller can build a fetch any more.

It used to take the promise, which puts the IPC call on the wire before the owner is reached — nothing left to coalesce, and a **held** `Alt+↑` or `Tab` was one `O(blocks-in-workspace)` scan per key repeat.
A burst is now one scan in flight plus one queued, whatever the repeat rate.
That module's doc owns the rules and the measurements; `lib/backlinks-fetch.test.ts` and `lib/backlink-handlers.cost.test.ts` are the nets (the second is invariant 15's *is it cheap once*, driven through the real `backlinkRowOps`).
**Mobile needs no coalescer:** `BacklinksSection.row-ops.ts` refuses a second gesture while one is in flight, `busy()` spans the mutation *and* the awaited re-ask, and a touch gesture has no key repeat.

**The section's own pending state had the mirror defect, and reserving space is not the fix.**
It mounted a 95px header + skeleton when the scan started and unmounted it when the answer came back empty — on a workspace where most journal days have no backlinks, a blink on most journal days, re-armed by every peer refresh.
A reservation that collapses still blinks, and one that does not is permanent dead space under every page.
`lateSectionVisible` (`@outl/shared/outline`) owns the rule and the measurement: a **first load draws nothing**.
A **refetch** keeps its rows — blanking them reads as "this page lost its references" — and is the one case that still shows the `loading…` mark.
Net: "the backlinks section never appears and then disappears" (`InlineBacklinks.test.tsx`).

**The ancestor breadcrumb is `<CrumbTrail />` (`@outl/shared/markdown`), never a `join` of `crumb.text`.**
A crumb carries inline tokens, so the trail renders like any other block and stays inert; the plain join survives only as the `title=` tooltip, through `crumbTrailText`.
Mobile's `BacklinksSection` wraps the same component, which is what stops one client showing markdown source while the other renders it.

### The cursor lives in one region

**`backlinkCursor()` (`lib/cursor-region.ts`, over that module's `backlinkRowIds()`) is the single owner of "is the cursor in this section", and every reader goes through it — the row's own highlight and `isEditing()` included.**
`selectedBlockId` and `selectedBacklinkBlockId` are declared mutually exclusive because each reader checks the backlink one *first*.
With both set, `j` walks someone else's page and `Enter` navigates away instead of editing, while the outline still paints the block the user left the cursor on.
That exclusion was a sentence: ~20 call sites write `selectedBlockId` directly and each had to clear the other field, and most did not.
The outline wins because releasing it is the deliberate act — the two handlers that cross the separator null it in the same breath, so no other writer has to know the rule exists.
A stale id painting a second accent bar is the visible half of the same bug, which is why the highlight may not read the raw field either.

**The highlight reads it through a `createMemo`, not per row.**
`backlinkCursor()` asks whether the section is on screen, which includes `zoomView()` — `focusSubtree(outline, focusBlockId)`, an O(page) walk — whenever `focusBlockId` names a block the outline has lost.
That is the one render before `<OutlineView />`'s self-heal effect clears it, and per row it cost 851 ms against 166 ms to mount 1,556 rows over a 688-block page, and 724.7 ms against 2.6 ms for one cursor move.
Same shape as `focus()` and `visualSet`; `InlineBacklinks.cursor-cost.test.tsx` counts the walks rather than timing them.

**A pointer crossing the section is not a cursor move**, so a backlink row has no `onMouseEnter`.
It wrote the keyboard's own cursor, which is what made the bug read as intermittent: the user never aimed the gesture.
The hover affordance is the row's `hover:bg-…`, which costs no state.

This is what the TUI gets from `Focus` being an enum; its one path that still has to *assign* `Focus::Outline` is a mouse click, the same gesture that broke here.
Net: `lib/cursor-region.test.tsx`, plus `a_click_brings_the_cursor_back_from_the_backlinks` in `outl-tui`.

**Toggling a task from a backlink row (issue #144)** mutates the *source* page, and the backlink effect is keyed on the slug, which didn't change.
So `toggleBacklinkTodo` refetches `pageBacklinks(currentSlug)` itself — the exception that effect's comment calls out.
Never refresh it by re-opening via `openRef`: `PageView.backlinks` is always empty now, so that blanks the section.

### A backlink row is a place you can work

Every gesture that acts on an outline block acts on a backlink row too — the text edit, task state, `g D`, delete, indent, outdent, both moves, and create below / above — each addressed to the citing block's **own** page.
User-facing behaviour is [`docs/desktop-ux.md`](desktop-ux.md#backlinks-and-nested-pages-below-the-outline).
The cross-client contract is [`docs/client-backlinks.md` → Editing a backlink where it is cited](client-backlinks.md#editing-a-backlink-where-it-is-cited).

**`lib/backlink-handlers.ts` (`backlinkRowOps`) is the cross-page half, and it is not a spread group in `action-handlers.ts`.**
A group has to own its `Action` keys outright, and every key here is already `block-handlers.ts`'s for the outline case.
Two groups holding `IndentBlock` would make the spread order precedence, which that assembly's own comment says is readability.
So `block-handlers.ts` consults this module **first** and falls through on `false`, the shape `enterInsertOnBacklink` already had.
`false` means "the cursor is in the outline"; a **refusal** answers `true`, because falling through from a row acts on `appState.selectedBlockId`, which is `null` there by construction.

Five rules a contributor will trip on:

- **Never a `block.rs` command, for any of the ten gestures.**
  The cross-page family is `editBacklinkBlock` (text), `backlinkRowOp` (the seven chords) and `createBacklinkSibling` (`o` / `O`).
  A `block.rs` body acts on the page the user is looking at, so it may *tolerate* a stale id — three different ways, each of which reached the user from a row.
  `createBlock` appended an empty block at the source page's end and answered `Ok`.
  Six printed `ActionError`'s `Display`, a ULID, while the catalog's own sentence went uncalled (invariant 12).
  And `moveBlockUp` / `moveBlockDown` / `indentBlock` answered success while doing nothing.
  `commands/backlink_edit.rs` owns the argument and the one liveness gate behind all ten.
  Net: `lib/backlink-commands.test.ts` — a twin called **at all** from a row fails, whatever page id it was handed.
- **A refused commit keeps the row's editor up, on the user's draft.**
  The typed text exists in no op, in no `.md` and on no peer, only in that textarea, so closing it is the one loss here nothing can undo — and it is reachable whenever a peer trashes the citing block mid-edit.
  Leaving is still possible (a navigation, `Cmd+Shift+B`, a click into another row), and holding `editingBacklink` also keeps `isEditing()` true, so no background refresh remounts the row and re-seeds the draft.
  Same rule mobile ships; the net asserts the **text**, not that an error was reported.
- **The editing identity carries a page.**
  `appState.editingBacklink` is `{ pageId, blockId }`, a sibling of `editingBlockId` rather than a widening of it.
  `startBacklinkEdit` (`lib/cursor-region.ts`) is its one writer, and it releases the outline's editing id in the same breath: two mounted editors is two drafts, and the second commit overwrites the first.
- **The rows are re-asked per commit, never per keystroke — and a held chord is one scan in flight plus one queued.**
  `pageBacklinks` is an `O(blocks-in-workspace)` scan (invariant 15), and it is spent in three places: `settle()`, `BacklinkRowEditor`'s text commit (which has its own `backlinkCommitPlan` and does not call `settle`) and `toggleBacklinkTodo`.
  "Per commit" is not a bound on a **held** `Alt+↑` or `Tab`: every repeat is a commit, so the count is `trackBacklinks`'s to keep and not any of the three's to remember.
  What the reply may touch is `backlinkCommitPlan`'s answer, not this client's: a cross-page reply describes the *source* page, so it reaches the screen only when that page is the one on screen — which a page citing itself makes reachable.
- **Nothing here nudges.**
  Indent, outdent and move have no visible result on a flat row, and shipping them anyway is a recorded decision rather than a `Partial`, which is why `Capability::BacklinkInlineEdit` reads `Full` here and on mobile.
  The TUI's `Partial` on that row is a different gap: it has no gesture at all that opens a cited block's page, where this client's group header is a `<button>` that does.
  The two messages the module carries are for a row with no block to act on at all: an orphan backlink, and a cursor naming a row the list no longer holds.

**`o` / `O` on a row used to append a block to the page being read.**
`NewBlockBelow` / `NewBlockAbove` read `appState.page?.id` and had no backlink branch, so a gesture aimed at another page's block created an empty block at the end of the open one.
The branch comes **before** the `pageId` read now, and the new sibling is created on the source page; a self-citing page is the one case where it also enters Insert, because the block really is in the outline on screen.
Aiming it at the right page was only half of it: through `createBlock` a stale row id still appended at that page's end, which is the same defect one door along and why the first rule above exists.

## Properties (`key:: value`) — `PropertyEditor.tsx`

`components/PropertyEditor.tsx` replaced `@outl/shared`'s `<BlockProperties />` here, which edits an existing chip's value and nothing else.
Creating the first property therefore meant the TUI or the `.md`, and deleting was an invisible gesture (issue #13).
The three verbs it owns are in [`docs/desktop-ux.md`](desktop-ux.md#properties-key-value-chip-row).

The chord has no chip to click, so `AddProperty` writes `appState.addPropertyBlockId` and `<BlockRow />` opens a blank editor on that id.
`OpenProperties` (the TUI's `g p`) resolves to the same handler: that overlay exists to *list* a block's properties, and the chip row is already that list permanently, so the only half it lacks is the blank pair.
The editor calls `onAddOpenChange(false)` on close, which is what lets a second press re-open it on the same block.
The page-property row is hidden while zoomed — the header is a *block* then, and page metadata under a block title reads as the block's.

`propertyChips` stays in `@outl/shared/markdown` — wrapped, never reimplemented.
Promoting `PropertyEditor` itself waits on mobile: its answer to the same issue is a sheet with tappable key chips, not an inline row, so promoting a desktop-shaped API first would freeze the wrong contract.

## Page history — `TimelinePanel.tsx`

The panel (issue #241) and its two restore rows are in [`docs/desktop-ux.md`](desktop-ux.md#page-history-panel); `appState.timelineOpen` is the flag.

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

## Applying a `PageView` — `lib/page-view.ts`

`applyPageState(view)` is the **single writer** of what a reply says about the page: `page`, `pageProperties`, `parseWarnings`, the sticky `mdAheadOfLog`, the outline and a `projection_error`.
There used to be two `applyView`s, and the shell's wrote only `page` + outline.
That one is every page opened from the chrome (sidebar, picker, Today, a task row, a deep link), so each kept the previous page's property chips and its "this page isn't syncing" banner.
A caller adds only what is its own: cursor repair in the shell, auto-run and embeds in the outline.
The module doc owns the rest, including why the write order is load-bearing.
Net: `lib/page-view.test.ts`, which also pins by source-read that neither component writes those fields again.

## Which day it is

`<AppShell />` wires `createDayClock` (`@outl/shared/journal`) into `appState.todaySlug`, and **every surface that names a day reads that field**: `<PageTitle />`, the sidebar calendar's today ring, `<TasksPanel />`'s journal headings.

It exists because the desktop held two frozen copies of the day instead.
`<PageTitle />` had a `createResource` keyed on the page slug, so it re-asked on navigation and never otherwise; the sidebar calendar had a `new Date()` in a component body, read once at mount **and** on the browser clock rather than the configured `[calendar] timezone`.
A window left open overnight therefore called yesterday "Today" in the header, in the accent colour, over the page the user was about to write into, and ringed yesterday on the calendar.

Three rules a contributor will trip on:

- **The day comes from the backend, the schedule comes from the browser.**
  `today_slug` resolves in `outl_actions::clock`'s timezone; `nextDayCheckDelay` only decides *when to ask*, and is capped at an hour precisely because that schedule can be wrong.
  A `new Date()` used as the answer is the bug, not a shortcut.
- **Midnight relabels; it does not navigate.**
  The user may be looking at the window. Returning to today happens on the way *back* in (`focus` / `visibilitychange`), under `shouldWakeToToday` — the same rule the phone uses, so the two clients cannot disagree about when a session counts as "opening the app".
- **The clock is created in the component body**, not in `AppShell`'s `async onMount`: it registers an `onCleanup`, and the owner is gone once that body awaits.

Net: `PageTitle.test.tsx` → "stops calling it Today once the day rolls over under an untouched window", plus `day-clock.test.ts` in `@outl/shared`.

## Zoom / focus into a block

The chrome (page-like header, zoom path, which blocks render) is in [`docs/desktop-ux.md`](desktop-ux.md#zoom--focus-on-a-block), the cross-client contract in [`docs/client-editing.md`](client-editing.md#zoom--focus-on-a-block-roamworkflowy).

Five decisions beyond that contract:

- **`zoomView()` (`lib/store.ts`) is the one owner of "is this window zoomed, and into what".**
  Three surfaces have to agree — what the body renders, where `j` / `k` may go, and whether the page's own sections are drawn — and each used to resolve `focusBlockId` against the outline itself.
  A stale id read as zoomed by one and unzoomed by another is a cursor walking into a section nothing painted, which is exactly what the local `backlinkBlockIds()` did before `backlinkRowIds()` (`lib/cursor-region.ts`) owned it.
  `<OutlineView />` wraps it in the `focus()` memo so the walk runs once per render, and passes the answer down as `<PageSections zoomed>` rather than letting that component ask a second time.
- **Nothing belonging to the page renders while zoomed.**
  `<PageSections />` draws nothing (backlinks, nested pages, namespace mentions), the page-property row is already hidden, and `backlinkRowIds()` returns `[]` so `j` clamps at the zoom's last row.
  The reason is the one the property row carries: the header is a *block* there, so the page's references under a block title are a wrong answer, not merely a distraction.
  Net: `PageSections.test.tsx`, plus "j stays inside the zoom instead of entering the page's backlinks" (`action-handlers.zoom.test.ts`).
- **`resetForNavigation()` (`lib/page-change.ts`) is what a navigation drops, and it has two callers on purpose.**
  `installPageChangeEffect` only notices a page *id* change, so it cannot see the user asking for the page they are already on — `Cmd+J` while zoomed into a block of today's journal navigated to today and left the zoom standing, so the chord looked dead.
  The handlers go through `ctx.navigateTo` (`handler-context.ts`) and the shell's chrome through `AppShell`'s `openView`; both are `applyView` plus that reset, and **a failed open resets nothing**.
  `applyView` stays the one for everything that is not a navigation: a commit, a batch op, `refreshActivePage` (a peer reload keeps the page *and* the user's zoom), a quick capture into today, and a timeline restore.
  `RemindersPanel` calls the reset inside its own `jumpTo`, because one `applyView` prop cannot answer for both that and `markDone`.
  Net: "OpenToday leaves a zoom held on today's journal" and "a failed OpenToday keeps the zoom".
- **No zoom stack:** `ZoomOut` reads the current focus's breadcrumb — last crumb is the parent, empty breadcrumb means top-level so it exits to the full page.
- **Bullet gesture split**, so zoom cannot collide with task state: a `•` bullet zooms via `onFocusBlock`, a `▢` / `▨` / `▣` checkbox keeps its one-stop-per-click toggle, and the fold **chevron** stays collapse.
  DOING shares TODO's colour and is told apart by the half-filled glyph; only DONE strikes the body.
- **`j`/`k` stay inside the zoom:** `SelectionUp`/`SelectionDown` walk `navBlocks()` (`[fv.root]` when zoomed) so the cursor can't escape the subtree.

## Task list — `TasksPanel.tsx`

What the panel lists and how it opens: [`docs/desktop-ux.md`](desktop-ux.md#task-list-panel).
No longer mobile-only: `Capability::TaskList` is `Full` here, and only the TUI keeps the `query`-block nudge.
Order and grouping are the backend's — `outl_actions::tasks` decides, `groupTasksByPage` sections, and nothing here ranks.
