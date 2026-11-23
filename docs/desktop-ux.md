# Desktop UX behaviour

How `outl-desktop` behaves, feature by feature.

Moved out of `crates/outl-desktop/CLAUDE.md`, which is loaded into the model's context on every session in that crate and had grown past the point where that is useful.
These sections are reference: you read one when you touch that feature, not every time you open the crate.
The rules a contributor must not break stayed in `CLAUDE.md`.

Its mobile twin is [`docs/mobile-ux.md`](mobile-ux.md), and the cross-client contract behind most of these surfaces is [`docs/clients.md`](clients.md).
Chords are [`docs/shortcuts.md`](shortcuts.md); which client performs which action is [`docs/client-parity.md`](client-parity.md), generated from `outl_shortcuts::support`.
Anything visual — colours, tokens, spacing, components — is specified in [`DESIGN.md`](../DESIGN.md).
The behaviour rules these sections are instances of are in [`UX.md`](../UX.md).

## First run

`App.tsx` decides between `<Onboarding />` and `<AppShell />`:

- **Returning user** — the workspace already opens at boot (`currentWorkspace()` plus `workspaceStats().ready`), so they go straight to `<AppShell />`.
  `refresh()` also sets the onboarded flag silently for them, so they never see the flow.
- **First run**, or a workspace folder that was removed — `<Onboarding />`.

The flow is two honest steps, no filler:

1. **Storage** reuses the existing `<WorkspacePicker />` (folder pick via `tauri-plugin-dialog` → `set_workspace`).
   On pick it fires `onWorkspacePicked`, which re-runs `App`'s gate, and advances.
2. **Sync**, optional, is the shared `SYNC_STEP` copy (`@outl/shared/onboarding`) plus the real `<SyncPanel />`, so the user can pair right there or skip.
   A single device is first-class.

Only the chrome is desktop-local; why the copy and the panel are shared rather than rebuilt is in [`crates/outl-desktop/CLAUDE.md`](../crates/outl-desktop/CLAUDE.md) → "First-run onboarding".

## Switch workspace — File menu (macOS) and Settings button

The native menu is built in `src-tauri/src/menu.rs` and exists on **macOS only**.
Tauri installs a default menu on macOS alone, so Windows and Linux never had a menu bar, and adding one there would be new chrome nobody asked for.
On macOS the default's File menu held only "Close Window", so the menu is built here instead.
It keeps what the default offered (the app menu, Edit with undo / redo / cut / copy / paste / select all, View, Window, and File → Close Window) and adds **File → Switch Workspace…** above Close Window, plus the **Help** menu described below.

**Settings → Workspace → Switch…** is the cross-platform route, and the only one on Windows and Linux.
It runs the same `switchWorkspace` flow as the menu item, and closes the modal once the workspace actually changed (not on cancel or failure: the draft still holds the old `last_workspace`, and Save writes it back).
The helper line under the path names the File menu on macOS and says only "Use the button to change." elsewhere (`switchWorkspaceHint`, which reads `@tauri-apps/plugin-os`).
Each time the modal opens it re-reads `last_workspace`, so a switch made through the menu does not leave a stale path on screen.

Clicking the menu item picks nothing in Rust; it emits `menu://switch-workspace`.
`AppShell` listens (`onMenuSwitchWorkspace`) and runs `switchWorkspace` (`lib/switch-workspace.ts`), the same function the Settings button calls:

1. A folder dialog titled "Switch outl workspace".
   Cancelling does nothing.
2. `set_workspace(path)`, the command the first-run picker calls.
3. The page, selection, backlinks and other slices tied to the old workspace are cleared, then today's journal of the new workspace is loaded.
   A failure lands on the error toast and leaves the current page on screen.

The menu item does nothing while `<Onboarding />` or the loading screen is up, because the picker is itself the screen then.
It has no chord and no palette row: both would need a new `Action`, which every client has to declare a verdict for ([`client-parity.md`](client-parity.md)).

## About window and the Help menu (macOS)

**outl → About outl** opens a real window, not AppKit's standard panel.
The panel shows an icon, a name, a version and a block of credits text, and nothing in it is clickable, so a user who hit a bug learned their version number and no way to act on it.

The window (`src-tauri/src/about.rs`) loads the shipped bundle at `index.html?view=about`; `src/index.tsx` branches on that query before it mounts anything and renders `<AboutWindow />` instead of `<App />`.
It is 380×460, fixed, centred, with the macOS title bar left empty so the content starts at the top.
A second click focuses the window already open rather than stacking a copy, and `Esc` or `Cmd+W` closes it.

It carries the brand mark in the theme's accent, the name, the running version, one line describing the project, the three links below, and "MIT licensed · Avelino and contributors".
The version comes from `getVersion()` and is simply absent when that read fails.
A placeholder build number would be worse than none: the version is the one fact this window exists to state correctly.

**Help** holds the same three links:

| Item | Opens |
|---|---|
| outl on GitHub | <https://github.com/outlmd/outl> |
| Report a Problem | <https://github.com/outlmd/outl/issues> |
| Sponsor outl | <https://github.com/sponsors/avelino> |

The table itself is `src-tauri/src/links.rs`, and it is the only copy.
The menu builds its items from it, and the click router resolves a clicked id back through it.
`src/lib/links.ts` is a mirror, pinned to it by `the_frontend_mirror_matches_this_table`.
So the two surfaces cannot name the same destination differently, and adding a row reaches both.

The window opens during the boot replay and on the first-run picker too.
That is the reason it is a window and not a modal inside `<AppShell />`, which only mounts once a workspace is open.
Those are the two moments a user most wants a version number and an issue tracker.
`capabilities/about.json` scopes it to exactly what it does: read the version, close itself, open an `https://` URL.
Nothing in it touches the workspace.

Windows and Linux have neither surface, for the same reason they have no menu bar.
The gap is recorded as `Capability::AboutAndSupport` across the three clients in [`client-parity.md`](client-parity.md).

## Chrome toggle bar and quick capture

`components/ChromeToggleBar.tsx` is a fixed bottom-left cluster, mounted by `AppShell`, carrying two visible mirrors of chords.
They are the **sidebar toggle** (`◫`, mirrors `Cmd/Ctrl+Shift+E`) and the **shortcuts-help toggle** (`?`, mirrors `?` / `Cmd/Ctrl+/`).
It floats over the main pane on an elevated surface, with the active toggle inverted to accent, so the sidebar button stays reachable while the left pane is hidden.

Quick capture is a box (`components/QuickCapture.tsx`) over state in `lib/quick-capture.ts`.
The OS-wide chord is registered in `src-tauri/src/quick_capture.rs`, which emits `quick-capture://open`.
A failed write keeps the text in the box.

The `Cmd+P` palette also lists actions that have no chord at all, from `CHORDLESS_ACTIONS` in `lib/palette-results.ts`, and splits task hits (shared `taskState`) out of block hits.

## Sync status dot

`<SyncIndicator />` sits in the top-right capsule of `<ChromeToggleBar />`, right before Settings, so the mesh state is glanceable without opening Settings.

The capsule mirrors the mobile top bar (`outl-mobile/src/components/JournalChrome.tsx`): fold all, unfold all, plugin toolbar buttons, reminders, plugins, sync dot, settings — same icons, same order.
Reminders is the one exception: mobile moved it to its bottom bar's Reminders tab (#364) and has no header button, so there is no twin to match.
The sidebar toggle has no mobile twin either and sits alone top-left.
The sidebar pads its own top so the capsule never covers search, and the page header (`OutlineView`) pads its top the same way so the eyebrow's attach and history icons clear the right capsule.

Dot states: green when at least one device is reachable, orange when none are, dim while the first probe runs, and a spinner while a sync pass runs.
**The state, its sentence and its colour are `@outl/shared/peers`' `syncState` / `syncStateLabel` / `syncStateColor`, and the glyph is that module's `<SyncDot>`** — the same four states, decided once, for this client and mobile ([#575](https://github.com/outlmd/outl/issues/575)).
Reachability counts both directions: a device that can only dial *us* (a phone behind carrier NAT) is reachable, and the responder records it on every confirmed inbound exchange.
The tooltip shows the reachability state and when this window last saw a sync land (a finished pass, or a `peer-ops-changed`).
Clicking syncs now — `syncNow()` then `reloadWorkspace()`, the same pass as the Sync panel's Refresh — and clicks during a pass are ignored.
Right-click opens Settings → Sync.
It re-probes on a slow interval and immediately on `peer-ops-changed`.

## Sidebar — hover `×` to delete a page

`Sidebar.tsx`'s `<Row>` takes an optional `onDelete` callback; when it is provided, a `×` button appears on hover.
`handleDelete(p)` calls `window.confirm(...)`, then `deletePage(slug)` (from `@outl/shared/api/commands`), applies the returned today's-journal view, and refetches the page list.
Journals are excluded, so only regular pages show the affordance.

The `g d` chord ("go delete", Normal mode) routes through the `DeletePage` handler and runs the same flow.
The backend is the shared `outl_tauri_shared::commands::page::delete_page` body, with no desktop-specific logic.

## Renaming and merging a page

**The page title is the rename door.**
Click it and it becomes an input, prefilled with the current name and selected, so typing replaces it.
Enter commits, Escape restores, and clicking away commits.
Submitting the name the page already has runs no command at all.
A **journal title is not clickable** and shows no affordance: its slug is its date, so the rename is refused, and offering a door only to refuse it is the defect rather than the guard.
The affordance is also absent while zoomed into a block, where the heading belongs to the block rather than to the page.

While the rename is in flight the title stays exactly where it is with a small pulsing dot beside it.
The outline and the editor are never disabled, unmounted or put in a load state.
A rename is a workspace-wide link rewrite, and none of that work is the page you are typing in ([`UX.md` → Never block a keystroke](../UX.md#2-never-block-a-keystroke)).

Two more page actions sit beside the sidebar's delete `×`: **Rename page...** and **Merge into...**.
Journals are excluded there too, so only regular pages show them.
All of them call the shared `outl_actions` functions through the `rename_page` / `merge_pages` Tauri commands, so every link to the page is rewritten in the same batch as the move ([RFC 0557](rfcs/0557-page-merge-and-rename.md)).
The dialog behind the sidebar's two icons is mounted at the shell rather than inside the sidebar, so it no longer depends on a panel that opens closed.

- **Rename** from either door offers the merge when the name is taken.
  Nothing is written yet, and a dialog asks whether to merge this page into the one that holds the name.
  Accepting runs the merge, cancelling leaves both pages as they were.
- **Merge into...** opens the page picker, which leaves out the page itself.
  It always asks for a confirmation before anything is written, because a merge has no undo.
- **Afterwards** the app opens the resulting page (the renamed one, or the merge target) and shows a summary toast: blocks moved, links updated, and one line per property both pages held with different values (the target's value is kept, the dropped one is named).

There is **no undo** for either action, and a merge in particular does not come apart.
The blocks stay under the target, the links stay rewritten and the old name stays as an alias: see [RFC 0557 → The opposite direction](rfcs/0557-page-merge-and-rename.md#the-opposite-direction).

The two actions have no chord on any client.
`Capability::PageRename` and `Capability::PageMerge` record the verdicts in [client-parity.md](client-parity.md): rename is missing in the TUI, merge is full on every client.
The warning about an alias that another page shadows (`alias_conflicts`) is deliberately not part of this change, so no GUI client shows it yet.

## Backlinks and nested pages below the outline

`components/PageSections.tsx` is what `OutlineView` mounts below the outline: `<InlineBacklinks />`, then `<NestedPages />` (`@outl/shared/namespace`, issue #275).

`InlineBacklinks.tsx`'s header direction button calls `setBacklinksOrder` to flip newest / oldest; `appState.backlinksOrder` hydrates at boot.
Contract: [`client-backlinks.md` → Backlinks order](client-backlinks.md#backlinks-order-issue-142).

The scan lands after the first paint, so **the section is on screen only once it has rows**.
A first load draws nothing at all, and the section then appears once, complete, below the last block.
A page with no references never draws anything, so there is no placeholder to take away and no reserved gap left under the page either.
A **refetch** is the opposite case and keeps everything it has, marked `loading…` beside the count — that is the one moment the mark is owed, because the rows are up and about to change.
`lateSectionVisible` (`@outl/shared/outline`) owns that rule, the measurement behind it and what each case costs.

A backlink row carries its own `▢` / `▣` / `•` indicator ahead of the text.
Clicking the indicator toggles the **source** block's TODO without leaving the page.
They are two buttons in one flex row, so the gestures cannot collide.

**Clicking the text edits the block in place**, in the same editor the outline uses — the chords and the four autocomplete popups come with it.
The two paste routes do not: they live in `commands/block.rs`, where a stale block id is re-targeted, which is the recovery `edit_backlink_block` exists to refuse, so a paste is the webview's literal splice and commits through the one audited command.
`i` and `Enter` on the row do the same thing, and `Esc` commits and returns to the row.
Every structural chord lands on that block on **its own** page: `Cmd+T` / `g D`, `dd`, `Tab` / `Shift+Tab`, `Alt+↑` / `Alt+↓`, and `o` / `O`.
A commit writes to a page you are not looking at, so nothing moves in the outline to confirm it: the row keeps its place and shows the committed text (backlink rows carry no saved mark, which is an outline-row affordance).

The row used to **navigate** instead, and the source page is still one click away: the group header is the page's title, and it is a `<button>`, so `F6` and the arrows reach it without a chord to memorise.
`o` / `O` creates the sibling on the source page and says so, because the new block cites nothing and no row here will ever show it — unless the page cites *itself*, in which case the block is in the outline on screen and the cursor lands in it.
A row whose citing block lives outside any page is inert: there is nothing to write to and nothing to open.

Contract, including why a cross-page edit **refuses** where an ordinary one re-targets: [`client-backlinks.md` → Editing a backlink where it is cited](client-backlinks.md#editing-a-backlink-where-it-is-cited).

**Hovering a row highlights it and nothing else.**
`j` / `k` cross into this section from the outline's last block, and the row holding that cursor draws the same accent bar a selected block does — so a pointer merely passing over the section on its way elsewhere must not move it.
It used to: `onMouseEnter` wrote the keyboard's own cursor, which took `j` and `Enter` off the block the user had left them on, from a gesture they never aimed.
The highlight is CSS now, and which row owns the cursor is `backlinkCursor()`'s answer alone ([`desktop-page-surface.md`](desktop-page-surface.md#the-cursor-lives-in-one-region)).

## Properties (`key:: value`) chip row

`components/PropertyEditor.tsx` renders the chip row and owns three verbs:

- **Create** — `+ prop` at the end of the row, revealed by the block row's `.group` hover, or `Cmd/Ctrl+Shift+P` in Normal mode.
- **Delete** — `×` on each chip.
  Same backend call as emptying the value; the button exists because the gesture was undiscoverable.
- **Key autocomplete** — `known_property_keys` (→ `outl_actions::known_keys`), most-used first, minus the keys the block already carries.
  Fetched on each open and never cached: the backend answer is a map scan, and a cached list is wrong the first time the user adds a key.
  Navigation goes through `handlePopupNav`, the same contract the `[[` and `/` popups use.

**Page properties** use the same component under the page title (`OutlineView.tsx`), fed by `appState.pageProperties` ← `PageView.page_properties` and written through `set_page_property`.
It passes `addAffordance="always"` there, because the header has one row and hiding its `+` behind hover would reintroduce the undiscoverable gesture.

Full cross-client table: [`shortcuts.md` → Properties](shortcuts.md#properties-key-value).

## Page history panel

The `⏱` button in the page-header eyebrow opens `components/TimelinePanel.tsx`: what the op log says happened to this page, newest first (issue #241).
`Esc` or a click outside closes it, and `appState.timelineOpen` is the flag.

Edits render as a word diff (`@outl/shared/trust`'s `DiffText`).
Two kinds of row carry a one-click restore, both decided by `restorePlan`: an edit puts the block's previous text back (`edit_block`), and a deletion brings the lost text back as a new block at the end of the page (`create_block`).
Moves, property changes, and an edit on a block that is now in the trash get no button — a deletion row offers that text back instead.

Contract and what a history counts as an event: [`client-page-lifecycle.md` → Page history](client-page-lifecycle.md#page-history).

## Trash panel

The `🗑` button in the chrome toggle bar opens `components/TrashPanel.tsx`: every block deleted out of this workspace, from any device, on any day (issue #287).
`Esc` or a click outside closes it, and `appState.trashOpen` is the flag.

Each row says what the block held, how long ago it went, and how many blocks come back with it — a deleted subtree is one row, not one per block.
**Restore** puts it back under the block it was deleted from and opens the page it landed on, so you see where it went rather than being told it came back.

A row that cannot come back says why instead of offering the button: a deleted page (its slug is usually taken by now), or a block whose parent went to the trash after it did.
That sentence is the backend's, so it reads the same here, in `outl trash list` and in the `outl_trash_*` MCP tools.

**A refusal can also arrive on a row that looked fine**, because the verdict is computed against the live tree and the list is a snapshot — another device can trash the landing parent in between.
The panel puts that sentence in the row rather than in a toast, keeps itself open, and re-reads the listing so the row settles into the state it should have been in.

No chord yet.
A chord needs an `Action` in `outl-shortcuts`, whose per-client `support()` match is at its frozen size baseline, which is the same reason the page-history panel has none.
Per-client reach is `Capability::Trash` in [`client-parity.md`](client-parity.md).

Contract, the three refusals and why a page cannot come back: [`client-page-lifecycle.md` → Restoring a deleted block](client-page-lifecycle.md#restoring-a-deleted-block).

## Blockquote chrome

A `"> "`-prefixed block renders with a left border and a ~6% tint, right-rounded, body at full colour; the outline bullet stays outside the quote chrome.
Detection is `splitQuote` plus `stripQuoteFromTokens`.

**`/quote` toggles it**, in the inline slash menu, on the block the slash was typed in.
The route is `blockSlashCommands` (`lib/slash-commands.ts`) → `createNativeSlashOps` (`components/outline-native-slash.ts`) → `toggleQuote` → the registered `toggle_quote` command → `outl_actions::block::toggle_quote`.
Typing `> ` at the front of the block still works and is the only route on the other two clients.

The row rides a reserved sentinel `plugin_id` (`@outl/block`), the same escape hatch `/upload` and `/date-today` use for a core feature with no chord.
**There is no chord and cannot be one yet**: that needs a `ToggleQuote` `Action` in `outl-shortcuts`, whose `support()` match and `defaults.rs` are both at their recorded file-size baseline.
Same reason the trash and page-history panels have none.
So the per-client reach is `Capability::BlockquoteToggle` in [`client-parity.md`](client-parity.md), where the TUI and mobile read `❌` with the marker to type instead.
Convention: [`client-editing.md` → Blockquote convention](client-editing.md#blockquote-convention).

## Zoom / focus on a block

Click a neutral `•` bullet, or fire `ZoomIn` (`z i` / `Cmd/Ctrl+Shift+]`), to zoom into the selected block; `ZoomOut` (`z o` / `Cmd/Ctrl+Shift+[`) pops one level.

The header becomes the focused block's own **page-like header**, Roam-style, with the block's text as the `<h1>` title.
The title is **rendered markdown** (`<ZoomTitle />`), not the raw source: a `#tag`, a `[[ref]]` and a `[label](url)` read as they do in the row the user zoomed out of, and the ref and the tag navigate from there too.
The eyebrow is a clickable **zoom path**: a leading page crumb (page glyph plus `<slug>` or `<title>`) exits the zoom back to the journal or page, then one crumb per ancestor re-focuses it.
The outline body renders the focused block's **children** (`rootBlocks()` → `fv.root.children`), so the block is not duplicated as both title and first row.
`navBlocks()` traverses the same children, so `j` / `k` stay in the body.
`addFirstBlock()` creates into the focused block (`parentId: focusBlockId`) when it is a zoomed-into leaf.

**Nothing that belongs to the page is on screen while zoomed.**
The page's `key:: value` row, the backlinks section, the nested-pages list and the namespace mentions all disappear, because the header is a *block* then and the page's references sitting under a block title read as that block's.
`j` off the last row of the zoom therefore stops instead of crossing into the backlinks, the way it clamps on an unzoomed page with the section toggled off.

**Opening a page leaves the zoom, including the page already open.**
`Cmd+J` zoomed into a block of today's journal returns to today's full journal; so do the sidebar's Today, a `Cmd+P` pick, a click on a `[[ref]]` or `#tag` that points back at this page, `[` / `]`, and an `outl://` deep link.
A commit, a batch op, a quick capture into today, a timeline restore and a peer-driven refresh all **keep** it — none of them is the user asking to go somewhere.

Cross-client contract: [`client-editing.md` → Zoom / focus on a block](client-editing.md#zoom--focus-on-a-block-roamworkflowy).

## Arrow keys walk between blocks in Insert mode

The outline reads as one continuous document while a block is open for editing, so the arrows do not stop at the block's boundary:

| Keystroke | Where the caret goes |
|---|---|
| `←` with the caret at offset 0 | end of the block above |
| `→` with the caret at the end | start of the block below |
| `↑` on the block's first line | block above, same column |
| `↓` on the block's last line | block below, same column |

The draft is committed on the way out, so the step is a save, not a loss.
Anywhere else inside the block the arrows are the browser's own motion, and a block with soft newlines (a fenced code block) is navigated internally first — `↑` only leaves once the caret is on its first line.

Three things it deliberately does not do.
A **modified** arrow is a different chord and never walks (`Alt+↑` / `Alt+↓` move the block; `Cmd+↑` / `Cmd+↓` are the OS's document-edge motion).
An arrow with text **selected** collapses the selection in place, like everywhere else.
And the walk **stops at the outline's edges** rather than stepping into the backlinks section below — those rows belong to another page, and opening one for editing from an arrow key aimed at this one is not what was pressed; at the top or bottom block the keystroke stays inside the block and nothing is committed.

A zoom is honoured: the walk covers the rows on screen, so the caret cannot leave a subtree the reader zoomed into.

The decision is `@outl/shared/caret-nav`, shared with mobile and mirroring the TUI, so the keystroke means the same thing on all three.
One shared limit: a *line* is a logical `\n` line, never a soft-wrapped visual one, so `↑` from the middle of a long wrapped block steps to the previous block instead of to the row above it.

## Inline autocomplete popups

`BlockEditor` hosts five popups, and only one is ever open at a time.

| Trigger | Popup | Source | What accepting inserts |
|---|---|---|---|
| `:shortcode` (word-initial `:[a-z]`) | `EmojiSuggestPopup` | `searchEmojis` → `outl_md::emoji::search` | the canonical `:shortcode:` |
| `[[…]]` | `RefSuggestPopup` | `search_pages`, the same command the `Cmd+P` picker calls | the page title, or the ISO slug for a journal |
| `((…))` | `BlockSuggestPopup` | `search_blocks`, from disk, debounced ~150 ms | the ref handle `((blk-XXXXXX))` |
| `#…` (word-initial, one word) | `RefSuggestPopup` | `search_pages` — a tag *is* a page | the page **slug**, never the title: `#` has no closer, so the first space would end the tag and leave the rest as prose |
| word-initial `/` | slash menu | this client's own actions (`actionSlashCommands`, from the `Cmd+K` palette's list), the text inserters (`insertSlashCommands`, from `outl_actions::insert`), plus `/upload`, the workspace's templates and every plugin command — all in `lib/slash-commands.ts` | a text inserter splices its answer over the `/token` and stays in the block; everything else runs the row: a native one through the dispatcher's handler, `instantiateTemplateAt` for `template: <name>`, the file picker for `upload`, else `pluginRun` |

**`/date-today` writes a date into the block; `/today` opens a journal.**
The first family is `outl_actions::insert` ([`docs/tui.md`](tui.md#date--time-inserters) owns the table), and accepting one splices its text over the `/token` without leaving the block or committing it.
`/date` is the one entry the desktop leaves out: the `/` trigger closes on the first space, so there is no way to type its argument here.

**A slash command is called the same thing here and in the TUI.**
The name comes from `outl_shortcuts::command_name`, which both clients read — `/today`, `/help`, `/run`, `/pin`, `/refer`, `/open`, `/quit`.
It used to be derived per client: the desktop kebab-cased the `Action` variant, the TUI carried a literal per built-in.
Of 31 TUI built-ins and 66 desktop action rows exactly two names coincided, so `/today` in the terminal was `/open-today` here.
An action the catalog does not name is left out of the menu rather than named locally.

Word-initial means the block's start **or** straight after whitespace, which is the TUI's rule — `nota /` opens the menu, `http://host` and `see docs/api` stay literal.
Detection and span replacement are `@outl/shared/autocomplete`'s (`detectRefContext`, `detectEmojiContext`, `detectSlashContext`, `applySuggestion`, `applyEmojiSuggestion`, `applySlashContext`), which the TUI and mobile run too.
When more than one trigger could fire, an open `[[`/`((` wins over `#` and `@`, and `#` wins over `@` — a tag is one word, so it is always the closer trigger when it matches.

`((blk-…))` handles also **resolve** at render (issue #147, TUI parity).
`OutlineView` gathers them with `collectBlockRefHandles` plus `resolveEmbeds` into `appState.embeds`, and `BlockBody` feeds that to `<MarkdownInline embeds= />`.
An inline `((blk))` renders the source text; an orphan renders a raw chip.
A block whose whole text is one embed (`embedOnlyHandle`) renders `<EmbeddedSubtree />` beneath it: read-only, depth 4, from `ResolvedBlock.children`.

In a `call:<name>` fence, `CodeFenceView`'s `CALL:<NAME>` chip links to the template page through `onOpenPage` → `openPageBySlug` (exact, not `openRef`), resolving the slug via `listTemplates()`; an unknown name is an inert chip.
Template contract: [`client-editing.md` → Structural templates](client-editing.md#structural-templates).

## Opening an external `[label](url)` link

`<MarkdownInline />` renders external links clickable through an `onLinkClick(href)` prop, and `OutlineView` wires it to `openExternalUrl`.
Failures land on the status line; ref and tag clicks still navigate; a backlink row edits in place, and only an orphan row stays inert.

An `assets/…` link routes to `openAsset` instead (via `isAssetLink`, `@outl/shared/links`), which opens it in the OS default app.
Two import paths share the backend `commands::asset`.
The `📎 Attach file` button (`@tauri-apps/plugin-dialog`) calls `attachAsset` and adds a new block at the end of the page.
Dragging a file onto a row calls the shared `installFileDrop` plus `importAssetFile` (`@outl/shared/drag-drop`) instead.
The link lands in the drop-on block — else the selection, else the block being edited via a caret splice, else a fresh end block.

## Task list panel

`<TasksPanel />` lists the workspace's open `TODO` / `DOING` blocks, grouped by page, over `list_open_tasks` — the command the phone's Tasks tab already read.
That command caps the rows at `TASK_LIMIT` (200) and reports the uncapped total, so the header reads `200 of 1431 open` rather than looking complete; the tail is not reachable from the panel.

It is built the way `RemindersPanel` is: mounted for the app's life, every `createResource` gated on `appState.tasksOpen`, `rovingKeydown` for arrows, `Esc` to close.
Open it with `g t` or `Cmd/Ctrl+Shift+L` (`OpenTaskList`); it is also in the `Cmd+P` palette.
A row opens through `openPageBySlug`, selects the block, and zooms into it when it has children — the same rule a block hit in the picker follows.

## Settings modal

Settings are stored through `outl-config` at the shared global config path (`~/.config/outl/config.toml` on macOS and Linux, the platform config directory on Windows).
What the modal exchanges with the backend is a flat JSON DTO from `src-tauri/src/settings.rs::Settings`:

```jsonc
{
  "last_workspace": "/Users/me/iCloud/outl",
  "vim_mode": false,
  "theme": "outl-light",
  "theme_dark": "outl",
  "theme_mode": "auto",       // "light" | "dark" | "auto"
  "font_size": 15,
  "sync_transport": "iroh",  // "iroh" (P2P, default) | "file" (iCloud/fs)
  "backlinks_order": "newest",  // "newest" (default) | "oldest" — read-only, see below
  "diagrams": "pele"  // "pele" (default) | "off". Mermaid fences. Mobile writes the same key.
}
```

`settings.rs` maps `sync_transport` to and from `[sync] transport` and preserves `relay_url` on save; it takes effect on next launch.
`backlinks_order` rides the DTO but is read-only here; the rule behind that, and the one about unmodeled sections, are in [`crates/outl-desktop/CLAUDE.md`](../crates/outl-desktop/CLAUDE.md) → "Settings".
`theme`, `theme_dark` and `theme_mode` carry the complete RFC 0022 pair and are owned by the modal.
Live preview goes through the shared `installTheme`, Save installs the persisted reply, and Cancel or a backdrop click reinstalls the configuration captured on open.

The device's actor id sits beside them as `actor`, a plain ULID.
Switching workspaces does not rotate it.

## Paired devices

Paired devices live in `<workspace>/.outl/peers.json`, per graph, owned by `outl_sync_iroh::PeersStore`.
`commands/peers.rs` exposes two thin Tauri commands over it.
`outl_peer_list` → `Vec<PeerDto>` (`node_id`, `alias`, `added_at`) reads that file, or the default when it is absent.
`outl_peer_remove(id)` → `bool` removes every peer whose `node_id` starts with `id` as a prefix.

The path is resolved from `AppState::storage_root` via `outl_sync_iroh::workspace_peers_path` — the same per-graph location the CLI and the iroh transport read, **not** `~/.outl/` or the app config directory.
Each command runs `migrate_global_peers_if_absent` first, so a user with a legacy global list keeps their peers on first open.
Only `identity.key` stays global, under `~/.outl/`.

## Sync panel

`components/SyncPanel.tsx` is the "Sync" section of `SettingsModal`, and besides the always-on `<SyncIndicator />` dot it is the only place the desktop surfaces sync state.

The panel header shows the same dot as the capsule, from the same `peersOnline(statuses())` — states are listed once, under [Sync status dot](#sync-status-dot).
The **Refresh** button calls `forceSync()`: `syncNow()` to force a P2P pull, `reloadWorkspace()` to re-render, then `refresh()` to re-read the device list and health for the dots.
`syncNow` / `reloadWorkspace` failures land on `appState.lastError` but never block the status read.

`components/TrustSection.tsx` renders inside it, with two things.
Per-device op counts from `device_activity`, labelled "This device" or a short actor id and never a guessed name, since an actor is not a peer id.
And the one-click Markdown export (`export_markdown`, into a new folder next to the workspace).
Presentation is `@outl/shared/trust`.

## Motion, focus mode, first-run tips

`BlockRow` slides children in with `outl-unfold` only when `collapsed` flips to false (`createSettle`), so a page load never animates.
`SyncIndicator` pulses the dot once when a pass ends.

Focus mode is the capsule button after the sync dot, in the same slot and with the same glyph as mobile.
The capsules carry `outl-focus-hide`, and the outline column carries `outl-reading`.

`PageSections` renders `<FirstRunTips client="desktop" />` when `isBlankOutline(rootBlocks())`, and the empty-page fallback in `OutlineView` shows it too.
