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

Dot states: green when at least one iroh peer is reachable, orange when none are, dim while the first probe runs, and a spinner while a sync pass runs.
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

## Backlinks and nested pages below the outline

`components/PageSections.tsx` is what `OutlineView` mounts below the outline: `<InlineBacklinks />`, then `<NestedPages />` (`@outl/shared/namespace`, issue #275).

`InlineBacklinks.tsx`'s header direction button calls `setBacklinksOrder` to flip newest / oldest; `appState.backlinksOrder` hydrates at boot.
Contract: [`clients.md` → Backlinks order](clients.md#backlinks-order-issue-142).

A backlink row carries its own `▢` / `▣` / `•` indicator ahead of the text.
Clicking the indicator toggles the **source** block's TODO without leaving the page; clicking the text navigates.
They are two buttons in one flex row, so the gestures cannot collide.

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

Contract and what a history counts as an event: [`clients.md` → Page history](clients.md#page-history).

## Trash panel

The `🗑` button in the chrome toggle bar opens `components/TrashPanel.tsx`: every block deleted out of this workspace, from any device, on any day (issue #287).
`Esc` or a click outside closes it, and `appState.trashOpen` is the flag.

Each row says what the block held, how long ago it went, and how many blocks come back with it — a deleted subtree is one row, not one per block.
**Restore** puts it back under the block it was deleted from and opens the page it landed on, so you see where it went rather than being told it came back.

A row that cannot come back says why instead of offering the button: a deleted page (its slug is usually taken by now), or a block whose parent went to the trash after it did.
That sentence is the backend's, so it reads the same here, in `outl trash list` and in the `outl_trash_*` MCP tools.

**A refusal can also arrive on a row that looked fine**, because the verdict is computed against the live tree and the list is a snapshot — another device can trash the landing parent in between.
The panel puts that sentence in the row rather than in a toast, keeps itself open, and re-reads the listing so the row settles into the state it should have been in.

No chord yet. A chord needs an `Action` in `outl-shortcuts`, whose per-client `support()` match is at its frozen size baseline, which is the same reason the page-history panel has none.
Per-client reach is `Capability::Trash` in [`client-parity.md`](client-parity.md).

Contract, the three refusals and why a page cannot come back: [`clients.md` → Restoring a deleted block](clients.md#restoring-a-deleted-block).

## Blockquote chrome

A `"> "`-prefixed block renders with a left border and a ~6% tint, right-rounded, body at full colour; the outline bullet stays outside the quote chrome.
Detection is `splitQuote` plus `stripQuoteFromTokens`, and toggling routes `toggleQuote` → `toggle_quote` → `outl_actions::block::toggle_quote`.
Convention: [`clients.md` → Blockquote convention](clients.md#blockquote-convention).

## Zoom / focus on a block

Click a neutral `•` bullet, or fire `ZoomIn` (`z i` / `Cmd/Ctrl+Shift+]`), to zoom into the selected block; `ZoomOut` (`z o` / `Cmd/Ctrl+Shift+[`) pops one level.

The header becomes the focused block's own **page-like header**, Roam-style, with the block's text as the `<h1>` title.
The eyebrow is a clickable **zoom path**: a leading page crumb (page glyph plus `<slug>` or `<title>`) exits the zoom back to the journal or page, then one crumb per ancestor re-focuses it.
The outline body renders the focused block's **children** (`rootBlocks()` → `fv.root.children`), so the block is not duplicated as both title and first row.
`navBlocks()` traverses the same children, so `j` / `k` stay in the body.
`addFirstBlock()` creates into the focused block (`parentId: focusBlockId`) when it is a zoomed-into leaf.

Cross-client contract: [`clients.md` → Zoom / focus on a block](clients.md#zoom--focus-on-a-block-roamworkflowy).

## Inline autocomplete popups

`BlockEditor` hosts five popups, and only one is ever open at a time.

| Trigger | Popup | Source | What accepting inserts |
|---|---|---|---|
| `:shortcode` (word-initial `:[a-z]`) | `EmojiSuggestPopup` | `searchEmojis` → `outl_md::emoji::search` | the canonical `:shortcode:` |
| `[[…]]` | `RefSuggestPopup` | `search_pages`, the same command the `Cmd+P` picker calls | the page title, or the ISO slug for a journal |
| `((…))` | `BlockSuggestPopup` | `search_blocks`, from disk, debounced ~150 ms | the ref handle `((blk-XXXXXX))` |
| `#…` (word-initial, one word) | `RefSuggestPopup` | `search_pages` — a tag *is* a page | the page **slug**, never the title: `#` has no closer, so the first space would end the tag and leave the rest as prose |
| block-initial `/` | slash menu | `templateSlashCommands` (`lib/slash-commands.ts`) plus the shared command set | runs the row, e.g. `instantiateTemplateAt` for `template: <name>` |

Detection and span replacement are `@outl/shared/autocomplete`'s (`detectRefContext`, `detectEmojiContext`, `applySuggestion`, `applyEmojiSuggestion`), which the TUI and mobile run too.
When more than one trigger could fire, an open `[[`/`((` wins over `#` and `@`, and `#` wins over `@` — a tag is one word, so it is always the closer trigger when it matches.

`((blk-…))` handles also **resolve** at render (issue #147, TUI parity).
`OutlineView` gathers them with `collectBlockRefHandles` plus `resolveEmbeds` into `appState.embeds`, and `BlockBody` feeds that to `<MarkdownInline embeds= />`.
An inline `((blk))` renders the source text; an orphan renders a raw chip.
A block whose whole text is one embed (`embedOnlyHandle`) renders `<EmbeddedSubtree />` beneath it: read-only, depth 4, from `ResolvedBlock.children`.

In a `call:<name>` fence, `CodeFenceView`'s `CALL:<NAME>` chip links to the template page through `onOpenPage` → `openPageBySlug` (exact, not `openRef`), resolving the slug via `listTemplates()`; an unknown name is an inert chip.
Template contract: [`clients.md` → Structural templates](clients.md#structural-templates).

## Opening an external `[label](url)` link

`<MarkdownInline />` renders external links clickable through an `onLinkClick(href)` prop, and `OutlineView` wires it to `openExternalUrl`.
Failures land on the status line; ref and tag clicks still navigate, and backlink rows stay inert.

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
