# RFC 0362: The journal is a stream of days, and each day keeps its own undo

| | |
|---|---|
| **Status** | Draft |
| **Issue** | none yet. The number is the PR that carries this RFC, [#362](https://github.com/outlmd/outl/pull/362), which ships no stream code |
| **PR** | [#362](https://github.com/outlmd/outl/pull/362) |
| **Date** | 2026-10-11 |
| **Reference doc** | none yet; `UX.md` principle 5 (view state is local) and `crates/outl-desktop/CLAUDE.md` once built |
| **Invariant** | root `CLAUDE.md` invariant 7 (state that must converge goes through the op log), by staying out of it: everything here is local view state |
| **Guarded by** | none yet, this is a Draft. The tests the implementation owes are listed under "How it cannot regress" |
| **Field report** | [hugows on #364](https://github.com/outlmd/outl/issues/364#issuecomment-6199037945), who reads a daily stream in Reflect and reported travelling to the wrong day by accident. Three of the rules under "What we chose" are his |

## Why

A journal is read across days, and outl shows one day at a time.
To see what you wrote yesterday next to what you are writing today you press `Cmd+[`, read, press `Cmd+]`, and the context you wanted side by side is gone.
On mobile it is the same with the chevrons.
The journal is the page people open most, and it is the one page where the neighbouring pages are always relevant.

The desktop is built around that single page on purpose, and it shows:

- `appState.page` and `appState.outline` in `crates/outl-desktop/src/lib/store.ts` hold one page.
- `undoPage(pageId)` / `redoPage(pageId)` are scoped to one page.
- `selectedBlockId`, `editingBlockId` and the visual anchor assume they live in that one outline, and the vim motions walk it.
- Backlinks are fetched lazily for that one slug after the outline paints (RFC 0169's lazy policy).

So "show the previous days under today" is not a layout change. It changes what owns focus, undo, selection and scroll.

## What we chose

A **journal stream** on the desktop: today, then the previous N days that have content, in one scroll, each day rendered by the same outline component it uses today.

- **One owner for the stream:** a `JournalStream` component and a `journalStream` slice of the store holding an ordered list of day views, each `{ page, outline }`. `appState.page` stays the *focused* day, so everything that reads it today keeps working.
- **Focus moves the page.** Selecting or editing a block in a day makes that day the focused page. `Cmd+[` / `Cmd+]` scroll to the neighbouring day instead of replacing the view.
- **Scrolling never moves focus.**
  Focus moves on a caret or a click, and on `Cmd+[` / `Cmd+]`, which are explicit.
  A day drifting into view is not a decision the user made, and treating it as one is what makes a stream lose track of which day it is writing to.
- **The focused day is named on screen at all times**, never inferred from where the caret happens to be.
  A cue that only appears once two days are visible is a cue that arrives after the mistake.
- **The stream is opt-in**, and one day per screen stays the default on every client.
  A user who never turns it on never meets any of the failure modes below.
- **Undo stays per day.** `Cmd+Z` undoes on the focused day, because that is what `undo_page` already does and what a user editing yesterday expects. There is no stream-wide undo.
- **Vim motions stop at a day's edge.** `j` on the last block of today does not jump into yesterday. Crossing days is `Cmd+[` / `Cmd+]`, as now.
- **Days load lazily.** The stream asks for the next day when the previous one scrolls into view. Empty days are skipped, which needs a backend list of journal slugs with content (a new read-only command, no Op).
- **Backlinks per day,** collapsed, fetched when a day becomes focused.
- **Mobile** keeps one day per screen and gets swipe between days instead. A stream on a phone fights the keyboard and the swipe-to-indent gestures in `UX.md`.

## Why not the alternatives

**Render the stream as one virtual page.** Merging the days into one outline would give one undo stack and free vim motions across days, but a block's page would become ambiguous at every boundary, and an edit could land in the wrong `.md`. The per-page model is what keeps every write going to one file.

**A stream-wide undo.** It would need either a new op that groups ops across pages or a frontend log of which page each undo belongs to. The first is an op log change for a view convenience. The second reorders undo across pages behind the user's back. Per-day undo is what each day already has.

**Copy Reflect's continuous editor.** Reflect's daily stream works because its editor is one ProseMirror document per note and the stream is notes stacked. That is close to what this RFC does, minus the engine. Switching the engine would break the block model, the vim mode and markdown roundtrip.

**Do nothing, add a "previous day" peek panel.** Cheaper, but it is a second rendering of a page with its own rules for editing, and the next request is "let me edit in the peek".

## The opposite direction

What gets worse:

- **Memory and first paint.** Several outlines are mounted instead of one. Days must load lazily and unmount when far off screen, or a long scroll on a big workspace reproduces the boot problems RFC 0128 fixed.
- **Where the keyboard goes is less obvious**, and this is the stream's primary risk, not a polish item.
  With one page, every key acted on it.
  With a stream, a key acts on the focused day.
  hugows, who reads a daily stream in Reflect every day, reports the failure directly: the journal view is "too free", and he travels to the wrong day by accident all the time.
  That is a field report about this exact design, from someone living in it, which is why the three rules above are requirements and not cues.
  The phone already answers the same risk and shows what the answer costs: its day swipe is confined to the week strip (`WeekStrip.tsx`), because rows below it swipe sideways for their own actions.
  The stream owes the desktop an equivalent, and it does not have one yet.
- **Undo surprises.** Edit today, click into yesterday, press `Cmd+Z`: nothing happens to today. That is correct per day and still surprising. The cue above has to make it legible.
- **Sync arriving for an unfocused day** has to re-render that day without stealing focus or scroll. Today a peer change refreshes the one page you look at.

What it does not permit: nothing here writes anything that didn't write before. No new Op, no sidecar field, no markdown change. Which day is focused and how far you scrolled never leaves the device (`UX.md` principle 5).

## How it cannot regress

Owed by the implementation PR, none exist yet:

1. A rule in `crates/outl-desktop/CLAUDE.md`: a stream day is a page, every mutation goes through that day's `pageId`, and undo never crosses days.
2. Tests:
   - `an_edit_in_a_stream_day_writes_to_that_days_page`, so a boundary edit can never land in a neighbour.
   - `undo_in_the_stream_only_touches_the_focused_day`.
   - `vim_j_on_the_last_block_of_a_day_stays_in_that_day`.
   - `a_peer_change_to_an_unfocused_day_keeps_focus_and_scroll`.
   - `days_far_off_screen_unmount`, with a block count bound so memory cannot grow with scroll length.
   - `scrolling_a_day_into_view_does_not_move_focus`.
   - `the_stream_is_off_until_turned_on`.

## Trust you can see

The same PR makes the engine's guarantees visible.
Page history renders each edit as a word diff and offers a restore where one is safe: an edit's previous text goes back through `edit_block`, a deleted block's text comes back through `create_block`.
Each restore is new ops, never a rewind of the log, so it syncs and undoes like any edit; moves and property changes get no button because their "before" depends on neighbours.
The sync settings show what the op log holds from each actor (`outl_actions::device_activity`), labelled honestly, since an actor id is not a peer id.
`export_markdown` copies the clean `.md` files and assets into a new folder next to the workspace.
A block shows a short "saved" dot once `edit_block` resolves.
Merged concurrent edits are listed in the sync panel (desktop Sync settings, mobile Devices sheet) under "Edits merged from other devices". On each sync reload `merges::detect` compares the old and new trees: a new remote Edit whose HLC is older than this device's last Edit on the same block, where the block text changed, is a race the CRDT merged. Each entry shows a word diff of this device's text against the merged result and an Undo, which writes `edit_text` back to our text as new ops (so it syncs, and `Cmd+Z` reverts the undo). The log is device-local view state, newest first, capped at 50, and kept in `.outl/state/merges-<actor>.json` so it survives a restart; the file is named by actor so devices sharing a synced folder never write each other's log. The per-device pending floors live the same way in `.outl/state/sync-pending.json`, capped at 64 peers. Detection and the log live in `outl_actions::merges`, so the TUI shares them: `:sync` lists pending and merged edits, with `u` to undo.

Limit: without causal context in the op the engine cannot tell every race. A peer edit made after ours but delivered late, or one whose clock runs ahead, is not listed; it is still merged and still in page history. Guard tests: `a_racing_peer_edit_is_listed_with_both_texts`, `a_peer_edit_with_no_local_edit_is_not_a_merge`, `the_log_is_newest_first_and_capped`, `the_log_survives_a_restart_and_so_does_an_undo`, `floors_survive_a_restart`.

## Polish you can feel

Motion is short and only ever opacity or transform: children slide in when a block unfolds (never on page load), a drop target eases into its ring, and the sync dot pulses once when a pass ends. Under `prefers-reduced-motion` all of it is off and the end state shows at once.
Reading gets one rhythm on both clients (`outl-reading`: 1.6 line height, kerning and ligatures, a little air between blocks). Body text uses `fg`, which clears 4.5:1 on all ten presets, and a test pins that.
Focus mode fades the chrome and dims every row but the one being written, with a 68ch measure on desktop. It is a device preference, like scroll position. It ships as a button in both capsules and as `Action::ToggleFocusMode`: `Cmd/Ctrl+Shift+F` on the GUI clients, `z f` in the TUI, where it hides the header, banner, sidebar and status bar (`F` is find-backward there, and a terminal reports `Ctrl+Shift+F` as `Ctrl+F`).
A new user gets three tips (blocks, links, daily) on an empty or blank page, in the same words on both clients, dismissed once per device.
The last two differences follow the desktop: mobile renders refs, tags and links as underlined text (`MarkdownInline` `variant="inline"`) and the TODO box as a square. Touch keeps a 44pt target on both, an invisible hit area around each ref and a 44pt button around the box.

## Speed and flow

One box finds everything. The desktop palette (`Cmd/Ctrl+P`, or `Cmd/Ctrl+K` on `>` for actions) lists pages, then tasks, then blocks, then actions. A task is a block hit whose text carries a task marker, read by the shared `taskState`, the same reading the bullet toggle uses, so "task" means one thing everywhere.
Rare actions get no chord but are still found by name: `CHORDLESS_ACTIONS` lists them, starting with "Export as Markdown", which runs the same `export_markdown` the Settings card does.
Quick capture is one line into the end of today's journal through `create_block`, so it is an op like any typed block. Three doors open the box: the `QuickCapture` action (`Cmd/Ctrl+Shift+Space`), the same chord registered with the OS through `tauri-plugin-global-shortcut` so it works with the window in the background, and `outl://capture`. On iOS the link opens the existing capture sheet, which is step 1 of RFC 0363. A chord another app already owns is logged and skipped; the other doors still work.
`QuickCapture` and `ExportMarkdown` are new `Action`s with a verdict for all three clients in `outl_shortcuts::support`; the TUI is `Missing` for both, with the reason in `support/why.rs`.
Undo for sync and plugin writes has its own section below. Keyboard reach for the chrome (roving focus, F6 regions, Esc to the outline) lives in `crates/outl-desktop/src/lib/keyboard-reach.ts`.

## Undo across sync and plugins

Undo is a per-page stack of `.md` snapshots, and a restore goes back through `restore_page_md`, so every undo is new ops in the log.
Until now only this device's own commits pushed a snapshot. A peer reload that changed a page **dropped** that page's stack (`invalidate_changed_history`), because restoring a snapshot taken before the peer's edit would silently revert it. A plugin write pushed nothing at all.

**What we chose: an outside change is its own undo step.**

- **Sync.** When a reload is about to publish, every page touched by the newly arrived ops (their nodes mapped through `enclosing_page_id` in both trees), plus every page that already has a stack, is rendered in the old and the fresh tree. Where the two differ, the old render is pushed onto that page's stack instead of the stack being dropped. History stays linear in time: `edit1, edit2, peer edit` undoes as `peer edit`, then `edit2`, then `edit1`, so an older snapshot is only ever restored after the peer's edit was itself stepped back, explicitly. Pushing clears redo, the same way any new edit branches history.
- **Plugins.** `plugin_run` and `plugin_sync_hooks` render the page on screen before handing control to the plugin thread. If the plugin applied ops and that page's render changed, the before render is pushed.
- **No new Op, no compensating op type.** Undoing a peer's edit is a `restore_page_md` like any other undo, so it is ordinary ops from this device. Peers see an edit, not a rewind, and the CRDT converges as usual.

**Why not the alternatives.** Inverse ops per remote op would need an inverse for every `Op` variant, including moves whose "before" depends on siblings, and the undo would interleave with ops the user never saw. Keeping the old drop rule is safe but means one `Cmd+Z` after any sync does nothing, which is what the issue asks to fix. A global, cross-page undo would reorder history behind the user's back (see "A stream-wide undo" above).

**What gets worse.** A user who presses `Cmd+Z` right after a sync reverts the peer's edit, which is visible to that peer. That is the point, and it stays recoverable: redo, page history, and the peer's own op log all still hold it. A plugin that writes to pages other than the one on screen is not undoable from those pages; it needs the plugin host to report which pages a run touched, and is left for a follow-up. After a snapshot boot the "new ops" set can be wrong, which is why pages with a stack are always diffed too.

**Guarded by** `a_peer_edit_becomes_its_own_undo_step`, `history_stays_linear_across_a_peer_edit`, `a_page_only_the_peer_touched_gains_a_stack`, `no_baseline_still_drops_everything` and `a_plugin_write_to_the_page_on_screen_is_undoable`.

## Scope

Not covered: mobile day swipe (a separate, smaller change with no RFC need), the iOS quick capture outside the app beyond the `outl://capture` link (it writes from another process, which is an op log and actor question for RFC 0363), and any change to how journal pages are created or named (RFC 0107).
