# CLAUDE.md — outl-desktop

Tauri 2 desktop client (macOS, Linux, Windows).
Solid + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation delegates to `outl-actions`.

## Status

**Feature-complete v0.**
Outline edit, journal nav, picker, backlinks, `outl-exec` code blocks, FS watcher + auto-reload, settings, and `desktop.yml` CI are all in.
Signed bundles, Homebrew cask, and graph view ride on top.

## Layering

```text
outl-core                    (CRDT, op log, storage trait)
outl-md                      (.md parse/render, sidecar)
outl-actions                 (workspace operations + SyncEngine, shared with TUI + mobile)
   ↑
outl-desktop (this crate)
   ├── src-tauri/src/lib.rs  (Tauri commands: parse args → outl-actions → render)
   └── (frontend in ../src)  (Solid components, Tailwind, @outl/shared)
```

## Hard rule

**This crate adds no business logic.**
If a Tauri command does something that involves the workspace shape (edit, move, todo, journal render), it delegates to `outl-actions`.
If you find yourself writing a tree walk or an op-generating helper inside `src-tauri/src/lib.rs`, stop — move it to `outl-actions` instead.
The TUI and mobile clients need it too.

Same rule on the frontend: before writing a helper under `src/lib/`, check `@outl/shared` (see [`crates/outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md)).
The renderer for inline tokens, paste detection, ref autocomplete, DTO types, and shared Tauri command wrappers all live there.

What this crate **does** own:

- Path discovery (file picker via `tauri-plugin-dialog`; persisted in settings JSON; cross-platform default).
- Cross-platform FS watcher (`notify` crate) that signals the frontend when peer `ops-*.jsonl` files grow — replaces the `NSMetadataQuery`/`NSFileCoordinator` dance the mobile crate has to do for iOS.
- Desktop-only Tauri command surface (workspace picker, settings IO).
  The code-execution command (`run_code_block`) is a **thin adapter** — the orchestration (flat-DFS walk, `.md` path resolution, `outl-exec` invocation, DTO build) lives in `outl_actions::exec`, shared with mobile.
  The desktop adapter only parses NodeIds, locks the workspace, calls the action, and wraps the outcome with a refreshed `PageView`.
  Adding behaviour to `commands/exec.rs` is almost always a smell — promote it to `outl-actions` instead.
- Solid frontend with **3-pane layout** (Sidebar / OutlineView / BacklinksPanel) and **OS-standard keyboard shortcuts** (`Cmd+P`, `Cmd+J`, `Cmd+T`, `Cmd+Enter`, `Cmd+,`) plus optional vim mode.
- `AppState` holds a `ProjectionWriter` (async `.md`+sidecar writes) — see [`outl-tauri-shared/CLAUDE.md`](../outl-tauri-shared/CLAUDE.md).

## Layout

The frontend / `src-tauri` file tree (which component owns what, where each `lib/*.ts` helper lives) is in [`docs/development.md`](../../docs/development.md#desktop-crate-layout).

## First-run onboarding

`components/Onboarding.tsx` is the first-run flow. `App.tsx` decides between it and `<AppShell />`:

- **Returning user** — workspace already opens at boot (`currentWorkspace()` + `workspaceStats().ready`) → straight to `<AppShell />`. `refresh()` also silently sets the onboarded flag for them, so they never see the flow.
- **First run** (or workspace folder removed) → `<Onboarding />`.

The flow is two honest steps, no filler:

1. **Storage** — reuses the existing `<WorkspacePicker />` (folder pick via `tauri-plugin-dialog` → `set_workspace`).
   On pick it fires `onWorkspacePicked` (re-runs `App`'s gate) and advances.
2. **Sync (optional)** — the shared `SYNC_STEP` copy (`@outl/shared/onboarding`) + the existing `<SyncPanel />` so the user can pair right there, or skip.
   A single device is first-class.

The "has the user onboarded" flag is a **per-install UI flag in `localStorage`** (`outl.onboarded`), **not** workspace state — it deliberately does NOT go through the op log.
It is intentionally not in `settings.json` either, since `settings.last_workspace` is the only first-run signal the backend tracks.

The onboarding **copy** lives in `@outl/shared/onboarding` (identical to mobile); only the chrome is desktop-local.
Pairing is **not** reimplemented — `Onboarding` renders the real `<SyncPanel />`.

### Sync status dot (always-visible)

`<SyncIndicator />` sits in the bottom-left `<ChromeToggleBar />` cluster so the mesh state is glanceable without opening Settings.
Green = at least one iroh peer reachable, orange = none, dim = first probe still running; clicking opens Settings → Sync.
It derives reachability from `peerStatus()` → `peersOnline()` (`@outl/shared/peers`), the same source the Sync panel and the mobile dot use.
It re-probes on a slow interval and immediately on `peer-ops-changed`.
Do not add a second reachability path; `peersOnline` is the one owner.

### Sidebar page deletion

`Sidebar.tsx`'s `<Row>` takes an optional `onDelete` callback; when provided, a `×` button appears on hover.
`handleDelete(p)` calls `window.confirm(...)`, then `deletePage(slug)` (from `@outl/shared/api/commands`), applies the returned today's-journal view, and refetches the page list.
Journals are excluded — only regular pages show the affordance.
The `g d` chord (Normal mode, "go delete") routes through the `DeletePage` case in `action-handlers.ts`, running the same flow as the `×` button.
The backend `delete_page` Tauri command is the shared `outl_tauri_shared::commands::page::delete_page` body — no desktop-specific logic.

`InlineBacklinks.tsx`'s header direction button (`setBacklinksOrder`) flips newest/oldest; `appState.backlinksOrder` hydrates at boot.
`OutlineView.tsx` refetches backlinks via `createEffect(on(slug, …))` — per navigation, not per commit (the "Esc is slow" fix; rationale in the code comment).

### Toggling a task from a backlink row (issue #144)

A backlink row carries its own `▢` / `▣` / `•` indicator ahead of the text.
Clicking it toggles the **source** block's TODO (`toggleTodo`) without leaving the page; clicking the *text* still navigates.
Two buttons in one flex row, so the gestures can't collide — the bullet-vs-checkbox split again.

Refresh is the non-obvious part.
The mutation lands on the source page, and `OutlineView`'s backlink effect is keyed on the slug, which didn't change.
So `toggleBacklinkTodo` refetches `pageBacklinks(currentSlug)` itself — the exception that effect's comment calls out.
Never refresh it by re-opening via `openRef`: `PageView.backlinks` is always empty now, so that blanks the section.

## Properties (`key:: value`) — `PropertyEditor.tsx`

`components/PropertyEditor.tsx` renders a chip row and owns the three verbs.
It replaced `@outl/shared`'s `<BlockProperties />` on this client: that component edits an existing chip's value and offers nothing else, so creating the first property meant the TUI or the `.md`, and deleting was an invisible gesture (issue #13).

- **Create** — `+ prop` at the end of the row (revealed by the block row's `.group` hover), or `Cmd/Ctrl+Shift+P` in Normal mode.
- **Delete** — `×` on each chip. Same backend call as emptying the value; the button exists because the gesture was undiscoverable.
- **Key autocomplete** — `known_property_keys` (→ `outl_actions::known_keys`), most-used first, minus keys the block already carries. Fetched on each open, never cached: the backend answer is a map scan, and a cached list is wrong the first time the user adds a key. Navigation goes through `handlePopupNav`, the same contract the `[[` / `/` popups use.

The chord has no chip to click, so `AddProperty` (in `action-handlers.ts`) writes `appState.addPropertyBlockId` and `<BlockRow />` opens a blank editor on that id.
`OpenProperties` (the TUI's `g p`) resolves to the same handler: that overlay exists to *list* a block's properties, and the desktop renders that list permanently as the chip row — the only half it lacks is the blank pair.
The editor calls `onAddOpenChange(false)` when it closes, which is what lets a second press re-open it on the same block.

**Page properties** use the same component under the page title (`OutlineView.tsx`), fed by `appState.pageProperties` ← `PageView.page_properties`, written through `set_page_property`.
`addAffordance="always"` there: the header has one row, and hiding its `+` behind hover would reintroduce the undiscoverable gesture.
The row is hidden while zoomed — the header is a *block* then, and page metadata under a block title reads as the block's.

`propertyChips` (which keys are chrome, which get a glyph) stays in `@outl/shared/markdown` — one owner, wrapped, not reimplemented.
Promoting `PropertyEditor` itself to `@outl/shared` waits on mobile: its answer to the same issue is a sheet with tappable key chips, not an inline row, so promoting a desktop-shaped API first would freeze the wrong contract.

## Page history — `TimelinePanel.tsx`

The `⏱` button in the page-header eyebrow opens `components/TimelinePanel.tsx`: what the op log says happened to this page, newest first (issue #241).
Read-only, `Esc` or a click outside closes it, `appState.timelineOpen` is the flag.

Three things a contributor should not undo:

- **It is not the undo stack.** `Cmd+Z` walks this session's mutations (`commands/history.rs`, `outl_actions::history`); this walks every device's, from the start of the workspace. The panel and the command are named `timeline` precisely so the two don't get merged.
- **No restore button.** `outl recover` is the only restore path that exists, and it is narrow on purpose: it only writes back a revision that *contains* the current text as a prefix, so the write cannot drop anything. A general "put this revision back" needs its own safety argument; the panel deliberately does not assume one.
- **Nothing here decides what an event is.** Which blocks count as the page's (including the deleted ones), which ops are not events at all, and the order all come from `outl_actions::timeline`. `formatAt` renders `at_ms` and must never *re-sort* on it — the ordering is HLC with actor tiebreak and is already applied, so a device with a skewed clock can legitimately show a time that reads out of order.

The fetch is gated on `timelineOpen && appState.page`: the read walks every block in the page under the workspace lock, which is not something to do on every navigation on the chance the user might open the panel.

No chord yet — a global binding means an entry in `outl-shortcuts`, which every client consumes, and the TUI has no timeline surface to bind it to.

## Blockquote chrome

A `"> "`-prefixed block renders with a left border + ~6% tint, right-rounded, body full-colour; the outline bullet stays outside the quote chrome.
Detection is `splitQuote` + `stripQuoteFromTokens`; toggling routes `toggleQuote` → `toggle_quote` → `outl_actions::block::toggle_quote`.
Convention: [`docs/clients.md` → Blockquote convention](../../docs/clients.md#blockquote-convention).

## "This page isn't syncing" banner

`<PageAheadOfLogBanner client="desktop" />` (from `@outl/shared/warnings`) renders above the outline when `PageView.md_ahead_of_log` comes back set.
That means the page's `.md` holds lines the op log never recorded, so outl refuses to overwrite it and the page has stopped converging with the user's other devices.
The copy is owned by `@outl/shared/warnings::aheadOfLogNotice`, never written here; `client="desktop"` is what makes it name `outl reconcile --ahead-of-log` in the workspace folder.
`appState.mdAheadOfLog` is sticky per page (`stickyAheadOfLog` in `OutlineView.tsx`): only the open commands carry the flag, so a mutation reply would otherwise clear the banner on the user's first edit — the very action it warns against.
Sticky stops at `PageView.md_ahead_of_log_checked`: a reply that ran the check is authoritative in both directions, so the next open / refresh after `outl reconcile --ahead-of-log` clears the banner instead of leaving a healthy page marked as broken.
Convention: [`docs/clients.md` → Surfacing a page that stopped syncing](../../docs/clients.md#surfacing-a-page-that-stopped-syncing).

## Theme tokens

`src/lib/palette.ts::applyPaletteToRoot` writes the canonical `--color-outl-*` namespace plus the legacy `--color-ios-*` one `MarkdownInline` still consumes.
New desktop code uses only `--color-outl-*`.
Both namespaces, the boot defaults in `styles.css`, and the condition for deleting the legacy writes: [`docs/theming.md`](../../docs/theming.md#desktop-css-custom-property-namespaces).

## Running

Dev + bundle commands, and why Vite runs on port 1421: [`docs/development.md`](../../docs/development.md#desktop-tauri-2).

## Tests

Layers, tooling and the current frontend suites: [`docs/development.md`](../../docs/development.md#per-client-test-suites).

## Shortcuts

Catalog: **`crates/outl-shortcuts`** (single source of truth, also consumed by the TUI).
The desktop fetches it via the `list_shortcut_bindings` Tauri command on boot and wires every `Action` through `lib/action-handlers.ts`.

Two of these chords also have **visible icon affordances** in a fixed bottom-left cluster (`components/ChromeToggleBar.tsx`, mounted by `AppShell`):
the **sidebar toggle** (`◫`, mirrors `Cmd/Ctrl+Shift+E`) and the **shortcuts-help toggle** (`?`, mirrors `?` / `Cmd/Ctrl+/`).
They carry no business logic — clicking flips the same store signal the dispatcher flips, so button and keyboard stay in sync.
The cluster floats over the main pane on an elevated surface (active toggle inverts to accent) so the sidebar button stays reachable with the left pane hidden.

### OS-standard chrome and undo / redo

Chord table: [`docs/shortcuts.md`](../../docs/shortcuts.md).
Desktop-specific: `Cmd/Ctrl+Shift+X` runs the focused / selected code block (plain `Cmd/Ctrl+X` is OS cut / view-mode block cut).

### Undo / redo (Normal mode — fire when no textarea is focused)

Chords (`Cmd/Ctrl+Z` undo, `Cmd/Ctrl+Shift+Z` redo, `u` / `Ctrl+R` vim spelling) live in [`docs/shortcuts.md`](../../docs/shortcuts.md).
Deliberately **Normal**, not Global: with a textarea focused the chord falls through to the webview (the in-flight draft is the textarea's own undo domain), and a Global binding would `preventDefault` it away.
History is **block-level**: each mutation that goes through `finish_in_page` (edit, create, indent / outdent, move, delete, TODO / quote toggle, paste)
pushes the page's pre-mutation `.md` render onto a bounded per-page stack (`outl_actions::history::HistoryStacks`);
undo restores the snapshot through `outl_md::reconcile_md`, so the restore is itself ops in the log — the op log stays the source of truth, nothing is rewritten.
Fold toggles (`set_block_collapsed`) bypass `finish_in_page` and are not undoable, matching their "view state, not content" semantics.
Invalidation is **surgical**: a workspace **switch** clears every stack,
but a peer-driven **reload** (`peer-ops-changed` → `reload_workspace`) drops only the stacks of pages whose projection actually changed across the reload — restoring one of those would silently revert the peer's edits.
Pages the peer didn't touch keep their full undo depth.

### Inline markdown (Insert mode — fire when a textarea is focused)

`Cmd/Ctrl+B`/`I`/`E`/`Shift+X`/`K` wrap the selection (or insert the delimiter pair around the caret) — bold / italic / inline code / strikethrough / link.
Full chord + output table: [`docs/shortcuts.md`](../../docs/shortcuts.md).
Implementation lives in `lib/markdown-wrap.ts`: each handler reads `document.activeElement`, splices the value, dispatches an `input` event so `<BlockRow />`'s Solid signal stays in sync, then repositions the caret / selection.

### Paste (with and without formatting)

Behaviour + routing: [`docs/paste.md`](../../docs/paste.md).
Three guards (mobile mirrors them):

- Code-fence host bails `Cmd/Ctrl+V` to the native splice (`detectFence` early-return), keeping it literal.
- `Cmd/Ctrl+Shift+V` reads via `tauri-plugin-clipboard-manager` (`clipboard-manager:allow-read-text`), dodging the macOS webview "Paste" gate.
- Both pass `textarea.value` so `flushDraftBeforePaste` commits the draft first.

`create_block`: stale `after_id` (`NotInTree`) → append at page end (fixes `o`-key crash after peer reload).

### Block-editor chords (inside a block's textarea)

Chord table: [`docs/shortcuts.md`](../../docs/shortcuts.md).
Load-bearing notes a contributor needs:

- **Plain `Enter` → splits the block at the caret** (`onEnter`, issue #184).
  `Shift+Enter` → literal `\n` soft break (issue #119), handled in `BlockRow`'s `handleKeydown` (not the catalog; see the code comment).
- `Cmd/Ctrl+X` (cut) and `Cmd/Ctrl+Z` (undo) deliberately fall through to the webview — no catalog binding matches inside a textarea.
  Native per-keystroke undo is still broken: the controlled `value={draft()}` binding resets the textarea's undo stack on every keystroke (issue #80).
- Bracket auto-pairing (`[[`/`((` auto-close, `(`/`[`/`{` auto-pair with caret between, closer step-over, empty-pair collapse on `Backspace`) all live in `@outl/shared/autocomplete` (`autoPairBracket` / `autoDeletePair`, TUI parity).
- The four inline autocomplete popups (slash / emoji / block-ref / page-ref) share one keyboard contract via `handlePopupNav` (`lib/popup-nav.ts`, unit-tested): arrows cycle, `Enter`/`Tab` with no modifiers accept, `Esc` closes.
  `Shift+Tab` outdents on every popup now (the page-ref one used to accept it).

### Vim parity (Normal + Visual)

Chord list: [`docs/shortcuts.md`](../../docs/shortcuts.md) — don't duplicate it here.
This section captures only the **architectural decisions** a contributor needs to know before touching `lib/action-handlers.ts`.

- **Three categories of vim ops**, by what they need from the cursor model:
  1. **Block-level** (`a`, `A`, `S`, `Y`, `*`, `#`, `z R`, `z M`, `z z`, `V`, `g v`, `>` / `<` in Visual, `y` / `d` in Visual) — work on `selectedBlockId` or a range of block ids.
     **Implemented.**
  2. **Char-cursor in Normal** (`x` `X` `D` `C` `s` `r{ch}` `f{ch}` `F{ch}` `~` `e`) — need a character cursor inside the selected block.
     The desktop has no such cursor (only an id), so these handlers **surface a status-line nudge** pointing the user at `i` + textarea edits.
     Catalog entries stay so the help overlay shows them.
  3. **Pending-input** (`r{ch}`, `f{ch}`, `F{ch}`) — read a second character before applying.
     The dispatcher has no machinery for this today; categorised as char-cursor since they're blocked anyway.

- **Visual mode is real, and reachable without vim (issue #23)**: `Mode::"vim-visual"` in the store with `visualAnchorId` + `lastVisualRange`, painted at 18% accent opacity.
  `Shift+↓` / `Shift+↑` (`SelectRange{Down,Up}`, bound in **both** Normal and Visual) start or grow the range via the DOM "nothing focused" → Normal fallback, so one machinery serves vim and non-vim.
  `extendVisualRange` stays in the outline, never crossing into read-only backlinks.
  Every exit funnels through one `exitVisual()` (captures `lastVisualRange` for `g v`); resting `vim-normal` folds to `normal` in `detectMode` + `StatusBar`.

- **`<BatchToolbar />`** (`components/BatchToolbar.tsx`) floats `N selected` + Indent / Outdent / Move ↑↓ / Delete / Done while `mode === "vim-visual"`, firing the **same** `handlers` the keyboard does so button and chord can't drift.
  Its **Delete** confirms (`window.confirm`) when a selected block has children; keyboard `d` does not (vim convention).

- **Range ops walk bottom-up or top-down via `applyVisualBlockOp`, and tolerate id-already-gone.**
  `DeleteRange` + `MoveVisualRangeDown` iterate `[hi → lo]` (children before parents; a descending move clears the block below first); `IndentVisualRange` + `MoveVisualRangeUp` walk `[lo → hi]`.
  NodeIds are stable (`deleteBlock` is `Move(node, TRASH)`, moves preserve identity), so the highlight follows the re-render; `safeCall` swallows per-id failures.

- **`UnfoldAll` / `FoldAll` walk via `flattenAll` / `flattenParents`, never `flattenVisible`.**
  `zR` must expand subtrees hidden under a collapsed parent; a visible-only walk would no-op on every descendant of a folded node.
  `zM` (fold-all) uses `flattenParents` because `set_block_collapsed` always writes `Op::SetCollapsed` (the CRDT needs every flip to converge), so folding a leaf would make future children appear collapsed.
  `zR` (unfold-all) uses `flattenAll` (unfolding a leaf has no future effect).
  Mirrors `outl-tui`'s `collect_collapse_candidates` so the op count matches across clients.

- **`A` (`EnterInsertAtEnd`) routes through `appState.caretIntent`.**
  The handler sets `caretIntent: "end"` *before* `editingBlockId`; `<BlockRow />`'s `createEffect` reads it on mount, applies `setSelectionRange`, then clears the signal.

- **Visual highlight uses a memoised `Set<id>` at the parent, not a per-row predicate.**
  `<OutlineView />` builds `visualSet = createMemo(() => visualRangeSet(...))` once per change; `<BlockRow />` answers `props.visualSet?.has(id) ?? false` in O(1).
  The old per-row `isInVisualRange` (O(N²)/keystroke) is kept in `@outl/shared/outline` for tests only.

- **Char-cursor nudge is one shared handler:** the 10 char-cursor entries (`x` `X` `D` `C` `s` `r` `~` `e` `f` `F`) have **no handler at all**: `outl_shortcuts::support` marks them `Missing(why::NO_CHAR_CURSOR)` and the dispatcher shows the catalog's sentence, so the message can't drift.

- **`Y` / Visual `y` copy to the OS clipboard** via `copy_markdown` + `navigator.clipboard.writeText` (fills `yankRegister`; paste-in `p`/`P` deferred).

- **`NewBlockAbove` (`O`) uses `beforeId`, not a post-creation move walk.**
  `createBlock({ beforeId: anchor })` → `create_before` (floor-slot swap in core); never reintroduce the old create-at-tail + `moveBlockDown`-loop.
  `Cmd/Ctrl+Shift+Enter` is caret-aware in `BlockRow`'s keydown (col 0 → *before*, past col 0 → *below*); `stopImmediatePropagation` preempts the catalog's create-below binding.

- **Block clipboard: view-mode cut/copy/paste of a whole block** (chords: [`docs/shortcuts.md`](../../docs/shortcuts.md)).
  **Normal**-mode only; `appState.blockClipboard` = `{ kind: "cut", nodeId } | { kind: "copy", markdown }` (backend resolves the page via `enclosing_page_id`).
  Cut is one identity-preserving `Op::Move` (`block::move_after`, cross-page, self-subtree rejected); copy duplicates via `paste_block_after` with fresh ids.

### Zoom / focus into a block (Roam/Workflowy)

Click a neutral `•` bullet, or fire `ZoomIn` (`z i` / `Cmd/Ctrl+Shift+]`), to zoom into the selected block; `ZoomOut` (`z o` / `Cmd/Ctrl+Shift+[`) pops one level.
The header becomes the focused block's own **page-like header** (Roam-style).
The focused block's text is the `<h1>` title.
The eyebrow is a clickable **zoom path**: a leading page crumb (`📅 <slug>` / `📄 <title>`) exits the zoom back to the journal/page, then one crumb per ancestor re-focuses it.
The outline body renders the focused block's **children** (`rootBlocks()` → `fv.root.children`), so the block isn't duplicated as both title and first row.
`navBlocks()` traverses the same children so `j`/`k` stay in the body.
`addFirstBlock()` creates into the focused block (`parentId: focusBlockId`) when it's a zoomed-into leaf.

Load-bearing decisions:

- Zoom is **local view state, never an op** — `appState.focusBlockId` (default `null`), sliced at render time via `focusSubtree` (`@outl/shared/outline`).
  No Tauri round-trip, no `PageView` change; a display preference like `backlinksOrder`, not cross-device state.
- **No zoom stack:** `ZoomOut` reads the current focus's breadcrumb — last crumb is the parent, empty breadcrumb means top-level so it exits to the full page.
- **Bullet gesture split** (no task-state collision): a `•` (unmarked) bullet zooms via `onFocusBlock`; a `▢` / `▨` / `▣` checkbox (TODO / DOING / DONE) keeps its toggle, which walks one stop per click; the fold **chevron** stays collapse.
  DOING shares TODO's colour and is distinguished by the half-filled glyph; only DONE strikes the body.
- **Stale zoom self-heals:** `focusSubtree` → `null` when the id left the outline (peer delete / off-page move) clears `focusBlockId`; page navigation resets it too.
- **`j`/`k` stay inside the zoom:** `SelectionUp`/`SelectionDown` walk `navBlocks()` (`[fv.root]` when zoomed) so the cursor can't escape the subtree.

### `Enter` outside a textarea (Normal mode)

With a block selected and no textarea focused (the DOM fallback puts the dispatcher in Normal mode even with `vim_mode == false`),
`Enter` resolves to the shared `OpenRefUnderCursor` action — but the desktop handler **always enters Insert on the selected block**.
The one exception: when the selection sits on a **backlink row** (read-only), `Enter` opens the source page and lands the cursor on the referencing block.

Why diverge from the TUI: the TUI has a char cursor so "open the ref under cursor" is well-defined; the desktop only has a selected block, so **following a ref is the click on the token** (`onRefClick`) and `Enter` means edit.

### `:shortcode:` emoji autocomplete

Inside an open `:shortcode` trigger, `BlockRow` shows `EmojiSuggestPopup`, reusing `detectEmojiContext` / `applyEmojiSuggestion` and the `searchEmojis` command (backed by `outl_md::emoji::search`).
Accept inserts the canonical `:shortcode:` (the `.md` stores the literal, never the codepoint).
It beats the ref popup at the same caret (`detectEmojiContext` only fires on word-initial `:[a-z]`).

### `[[page]]` ref autocomplete

Inside an open `[[…]]`, `BlockRow` shows `RefSuggestPopup`, reusing the shared `detectRefContext` / `applySuggestion` helpers and the `search_pages` command the `Cmd+P` picker already calls.
Accept inserts the page title (or ISO slug for journals).

### `((block ref))` autocomplete + rendering

Inside an open `((…))` (issue #116), `BlockRow` shows `BlockSuggestPopup` — `detectRefContext` (`kind: "block"`) / `applySuggestion` + `search_blocks` (from disk, debounced ~150ms).
The pick inserts the **ref handle** (`((blk-XXXXXX))`), never the text; mobile registers it, popup unwired.
Rendering then **resolves** the handle (issue #147, TUI parity): `OutlineView` gathers handles via `collectBlockRefHandles` + `resolveEmbeds` into `appState.embeds`.
`BlockRow` feeds it to `<MarkdownInline embeds= />` (inline `((blk))` → source text, orphan → raw chip).
An embed-only block (`embedOnlyHandle`) renders `<EmbeddedSubtree />` beneath it — read-only, depth 4, from `ResolvedBlock.children` (`resolve_embeds`).

### Clicking external `[label](url)` links

`<MarkdownInline />` renders external links clickable via an `onLinkClick(href)` prop.
`OutlineView` wires it to `openExternalUrl` — scheme-guarded to `http(s)`/`mailto`, opened via **`tauri-plugin-opener`** (registered in `lib.rs`, capability `opener:allow-open-url`).
Failures land on the status line; ref/tag clicks still navigate, backlink rows stay inert.
An `assets/…` link routes to `openAsset` instead (via `isAssetLink`, `@outl/shared/links`) — opens in the OS default app.
Two import paths share the backend `commands::asset`: the `📎 Attach file` button (`@tauri-apps/plugin-dialog`) calls `attachAsset`, a new block at the end.
Dragging a file onto a row calls the shared `installFileDrop` + `importAssetFile` (`@outl/shared/drag-drop`) instead, landing the link in the drop-on block (else selection, else the block being edited via caret splice, else a fresh end block).

### `/template` slash entry

The block-initial `/` menu lists native `template: <name>` rows (`templateSlashCommands`, `lib/slash-commands.ts`) that `OutlineView` runs via `instantiateTemplateAt`.
Contract: [`docs/clients.md` → Structural templates](../../docs/clients.md#structural-templates).

In a `call:<name>` fence, `CodeFenceView`'s `CALL:<NAME>` chip links to the template page — `onOpenPage`→`openPageBySlug` (exact, not `openRef`), slug via `listTemplates()`; unknown name = inert chip.

## Reminders (`remind::`)

`<RemindersPanel />` lists them, `set_block_remind` authors a rule, and `<AppShell />` polls `deliver_due_reminders` every 30s.
Panel wiring, the chord choices and the `[reminders] enabled` default: [`docs/reminders.md`](../../docs/reminders.md#desktop-outl-desktop).
The one rule that matters here: **nothing about when a reminder fires is computed in the frontend** — the instants come from `outl_actions::reminders`, the labels from `@outl/shared`.

## Plugins

How the desktop hosts plugins — the effect layer, the toolbar and view surfaces:
[`docs/plugins.md`](../../docs/plugins.md) → "Desktop plugin surface".

## Logging

`run()` in `src-tauri/src/lib.rs` installs a `tracing_subscriber` fmt subscriber writing to **stderr** as its first step (before rustls / Tauri setup).
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`.
Running `cargo tauri dev` from a terminal then shows the iroh P2P transport's `info!`/`warn!`/`debug!` lines (endpoint bound + node id, each connect attempt's target + outcome, "delta sync received N ops") so device↔device sync is debuggable.
Init uses `.try_init()` so a double-init can't panic.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) for what the transport logs.

## Settings

Stored at `<app_config_dir>/settings.json`:

- macOS: `~/Library/Application Support/app.outl.desktop/`
- Linux: `~/.config/app.outl.desktop/`
- Windows: `%APPDATA%\app.outl.desktop\`

Schema (`crates/outl-desktop/src-tauri/src/settings.rs::Settings`):

```jsonc
{
  "last_workspace": "/Users/me/iCloud/outl",
  "vim_mode": false,
  "theme": "auto",       // "light" | "dark" | "auto"
  "font_size": 15,
  "sync_transport": "iroh",  // "iroh" (P2P, default) | "file" (iCloud/fs)
  "backlinks_order": "newest"  // "newest" (default) | "oldest" — read-only, see below
}
```

The Sync transport select in `SettingsModal` writes `sync_transport`.
`settings.rs` maps it to/from `[sync] transport` and preserves `relay_url` on save; takes effect on next launch.
`backlinks_order` is read-only here — `save` restores it from disk (same pattern as `[calendar]`) so the modal can't clobber the dedicated `set_backlinks_order` command's write.

The actor id (one per device) lives next to it as `actor` — a plain ULID.
Switching workspaces does not rotate it.

## Peers

Paired devices live in `<workspace>/.outl/peers.json` (per-graph), owned by `outl_sync_iroh::PeersStore` (see [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md)).
The desktop exposes two thin Tauri commands in `commands/peers.rs` — no business logic, they just load the store and project / mutate it:

| Command | Returns | Behaviour |
|---|---|---|
| `outl_peer_list` | `Vec<PeerDto>` (`node_id`, `alias`, `added_at`) | Loads `peers.json` (or default if absent) and lists every paired peer |
| `outl_peer_remove(id)` | `bool` | Removes peers whose `node_id` starts with `id` (prefix match); `true` if any were removed |

The path is `<workspace>/.outl/peers.json` (resolved from `AppState::storage_root` via `outl_sync_iroh::workspace_peers_path`) — the same per-graph location the CLI and the iroh transport read, not `~/.outl/` or `<app_config_dir>`.
Each command runs `migrate_global_peers_if_absent` first, so a user with a legacy global list keeps their peers on first open.
Only `identity.key` stays global (`~/.outl/`).

`commands/peers.rs` also exposes `outl_sync_now()` (reads `state.iroh_transport`, the `Arc<dyn SyncTransport>`, and calls the trait's `sync_now()`) — the force-sync trigger behind the Sync panel's Refresh.

### When this window does not hold the device endpoint

A device binds **one** iroh endpoint, and which process gets it is decided by a lease, not by being the GUI ([`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "One endpoint per identity, elected not assigned").
So the desktop can now be running with `iroh_transport` / `iroh_pairing` empty while sync works fine through the shared `ops/` dir — a co-resident `outl mcp serve` that started first is the ordinary case, since Claude Desktop launches it at login.

`iroh_sync::no_endpoint_reason()` separates the reasons those slots can be empty, because they deserve opposite answers: `NoEndpoint::P2pDisabled` (the user's own opt-out), `HeldByAnotherProcess` (lost the election), or `Unavailable(why)` (the lease could not be arbitrated, or the transport failed to build).
It is rewritten on every `wire_iroh_transport`, so a workspace swap that wins the endpoint clears the previous warning.
Only `P2pDisabled` is the user's choice, and it is the only one that refuses to pair; collapsing the others into it told a user whose transport failed to build to switch on a setting that was already on.

- **Pairing.**
  With no endpoint of its own, `outl_peer_pair_host` / `outl_peer_pair_join` fall back to `outl_sync_iroh::host_pairing` / `join_pairing` — the one-shot helpers the CLI uses, which bind their own endpoint and close it before returning.
  For the seconds that handshake runs it **does** take the relay route from the lease holder.
  Accepted deliberately: pairing is rare, explicitly user-initiated and short, the holder recovers when the one-shot closes, and the alternative is a user who cannot add a device at all.
  It is not a precedent — no other path in the GUI may bind an endpoint.
  When P2P is simply **off** (`transport = "file"`) pairing is **refused** instead, because binding there would override the setting the user picked on the one path where we know they are looking at the app.
- **Refresh.**
  `outl_sync_now` returns an error naming the reason (the holder, or what stopped this window from claiming the endpoint) rather than an `Ok(())` that did nothing.
  The silent no-op was the actual defect: the dot stays orange, Refresh appears to work, and nothing says why.
  `transport = "file"` stays a quiet no-op — the user's own choice is not a degraded state.
  Refresh also **re-runs the election** before reporting that (`retry_endpoint`): the recorded reason is a snapshot of login, the holder can exit at any time afterwards, and nothing in this process notices, so without a retry the window would explain a dead process for the rest of the session.
  It re-contends only from `HeldByAnotherProcess` / `Unavailable` — `P2pDisabled` is a setting rather than a race, and a `None` reason with no transport means the boot opener has not wired yet and a second pass would race it for the same lease.

**Still missing:** the status dot itself cannot tell "every device is offline" from "another local process holds the endpoint".
`PeerStatusDto` lives in `outl-tauri-shared` and `peersOnline` (`@outl/shared/peers`) is the one owner of reachability across desktop and mobile, so teaching it this state means a shared DTO change, not a desktop-local one.
Until then the Refresh error is the surface that explains it.

### Sync panel dot + refresh (iroh-driven)

`components/SyncPanel.tsx` (the "Sync" section of `SettingsModal`) is the only place the desktop surfaces sync state; there is **no** always-on chrome dot (`StatusBar` / `ChromeToggleBar` carry none).
The panel header shows a small status dot derived from the shared `peersOnline(statuses())` helper (`@outl/shared/peers`) — green when at least one iroh peer is reachable, orange when none are (no peers paired, or all unreachable).
The **Refresh** button calls `forceSync()`: `syncNow()` (force a P2P pull) → `reloadWorkspace()` (re-render) → `refresh()` (re-read the device list + health for the dots).
`syncNow` / `reloadWorkspace` failures land on `appState.lastError` but never block the status read.

**`reload_workspace` offloads the replay off the IPC thread.**
Its heavy half — `SyncEngine::reload_workspace`, a full O(all-ops) replay — runs in `tauri::async_runtime::spawn_blocking`, not inline on the IPC worker.
A synchronous command froze the window through the rebuild.
On iOS the same shape trips the scene-update watchdog (>10s → SIGKILL) after a big peer push, so mobile mirrors it.
Only the cheap tail (history invalidation + `Mutex` swap + reconcile spawn) runs on the command thread.
`syncNow()` + `peersOnline()` live in `@outl/shared` so desktop and mobile derive the dot + drive the refresh identically — see [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

## Deep links (`outl://`)

The scheme contract, the shared `outl_actions::parse_deep_link` parser, and this client's warm / cold wiring live in [`docs/deep-links.md`](../../docs/deep-links.md#desktop-wiring-outl-desktop).
One thing that bites every time: **testing on macOS needs a bundled, installed app** — LaunchServices only registers `outl://` from the built bundle, so `cargo tauri dev` never sees it.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-desktop --all-targets -- -D warnings`
3. `cargo test -p outl-desktop`
4. `bun --filter outl-desktop test` (Vitest)
5. `cd crates/outl-desktop && cargo tauri dev` — smoke open in a real window, click around the parts you touched.
6. If you touched anything in `@outl/shared`, also run `bun --filter outl-mobile test` to confirm paridade.
