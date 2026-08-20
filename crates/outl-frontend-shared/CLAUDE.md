# CLAUDE.md — outl-frontend-shared

The shared TypeScript + Solid library every outl frontend client (`outl-mobile`, future `outl-desktop`) consumes.
**Lives here so two clients never reimplement the same thing.**

## Why it exists

Mobile and desktop are different *shells* on top of the same Rust backend.
Most of the UI is genuinely client-specific (touch gestures vs. mouse + keyboard, single-pane vs. 3-pane chrome, OS menus), so the **shells stay isolated** in `crates/outl-mobile/src/` and `crates/outl-desktop/src/`.

But a handful of pieces are dumb pure logic the two clients need *identically*:

- The renderer that turns `InlineToken[]` into JSX.
- The "does this look like a markdown outline?" heuristic that mirrors `outl_actions::paste::looks_like_outline`.
- The caret-aware `[[…]]` / `((…))` detector that mirrors `outl_tui::overlay::detect_trigger`.
- UTF-16 ↔ codepoint offset conversion (textarea quirk).
- DTO interfaces the backend serialises (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink`, `InlineToken`, …).
- Typed `invoke()` wrappers for the Tauri commands every client calls (navigation, mutations, paste, collapsed).

Keeping these in a separate library is the Rust "Reuse-first" policy ([root CLAUDE.md](../../CLAUDE.md#reuse-first)) applied to TS — one owner, every client wraps.

## Layout

```
crates/outl-frontend-shared/
├── package.json            # name "@outl/shared", peerDeps solid-js + @tauri-apps/api
├── tsconfig.json
├── vitest.config.ts
└── src/
    ├── index.ts            # barrel re-export
    ├── api/
    │   ├── types.ts        # PageMeta, OutlineNode, BlockNode, Backlink, InlineToken, …
    │   └── commands.ts     # invoke<T>() wrappers for shared Tauri commands
    ├── warnings/
    │   ├── ParseWarningsBanner.tsx
    │   ├── PageAheadOfLogBanner.tsx
    │   ├── ahead-of-log.ts        # aheadOfLogNotice — the copy, one owner
    │   ├── ahead-of-log.test.ts
    │   ├── styles.css
    │   └── index.ts
    ├── markdown/
    │   ├── MarkdownInline.tsx
    │   └── index.ts
    ├── paste/
    │   ├── index.ts        # looksLikeOutline, utf16OffsetToCharOffset
    │   └── paste.test.ts
    ├── autocomplete/
    │   ├── index.ts        # autoClosePair, autoPairBracket, autoDeletePair, insertPair, insertText, detectRefContext, applySuggestion
    │   └── autocomplete.test.ts
    ├── onboarding/
    │   ├── index.ts        # first-run copy (STORAGE_STEP, SYNC_STEP, FINISH_CTA) — plain data, no invoke
    │   └── onboarding.test.ts
    ├── journal/
    │   ├── index.ts        # parseJournalSlug, formatJournalSlug, journalSlugToDate, daysInMonth, MONTH_NAMES, DAY_LABELS*, mondayIndex, prev/nextMonth
    │   └── journal.test.ts
    ├── outline/
    │   ├── index.ts        # rawTextWithTodo, findBlock, flattenNodes, countDescendants + id walks (flattenVisible/All/Parents, next/previousVisibleId, visualRange*)
    │   └── outline.test.ts
    ├── drag-drop/
    │   ├── index.ts          # installFileDrop, physicalToCss, blockIdFromElement/AtPhysical, joinAssetMarkdowns, appendMarkdownToBlock
    │   └── drag-drop.test.ts
    ├── plugins/
    │   └── transformer-registry.ts  # content-transformer registry + (blockId, body) result cache
    ├── toolbar/
    │   ├── actions.ts       # ToolbarAction catalog + TOOLBAR_META + DEFAULT_ORDER + PINNED_FIRST/LAST
    │   ├── mfu.ts           # most-frequently-used ordering (localStorage) — port of Swift ToolbarMFU
    │   ├── mfu.test.ts
    │   └── index.ts         # barrel (@outl/shared/toolbar)
    └── peers/
        ├── index.ts        # PairingQR, PeerList, ticketToSvg (barrel)
        ├── PairingQR.tsx    # ticket → scannable QR (owns its own encoding; no invoke)
        ├── PeerList.tsx     # pure list of paired devices (data + onRemove via props)
        ├── qr.ts            # ticketToSvg — pure ticket → SVG string (wraps `qrcode`)
        └── styles.css       # neutral baseline (@outl/shared/peers/styles)
```

## How clients consume it

```ts
// In a client component:
import type { Backlink, PageMeta } from "@outl/shared/api/types";
import { listPages, openRef } from "@outl/shared/api/commands";
import { MarkdownInline } from "@outl/shared/markdown";
import { looksLikeOutline } from "@outl/shared/paste";
import { autoClosePair, detectRefContext } from "@outl/shared/autocomplete";
```

Resolution happens through:

1. **Bun workspaces** (root `package.json` lists `crates/outl-frontend-shared` first).
   Bun dedupes `solid-js` and `@tauri-apps/api` across all clients — **critical for Solid**, because two copies of the framework in different `node_modules` directories silently break reactivity (signals diverge).
2. **`paths` in each client's `tsconfig.json`**:
   ```jsonc
   "paths": {
     "@outl/shared": ["../outl-frontend-shared/src/index.ts"],
     "@outl/shared/*": ["../outl-frontend-shared/src/*"]
   }
   ```
3. **`resolve.alias` in each client's `vite.config.ts` and `vitest.config.ts`** so Vite/HMR and Vitest resolve the same path the editor does.

## What enters the library

Decision rule (in order):

1. **Does the OTHER client also need it identically?**
   If yes, it goes here.
2. **Is it a pure function or stateless component?**
   If yes, it can go here.
3. **Is it the wire shape of something the Rust backend serialises?**
   If yes, it goes here as a type.
4. **Is the client shell tightly coupled to it (touch handlers, OS chrome, modes)?**
   Stays in the client.

When in doubt, ship in the client; promote later when the second client appears.
**Never** add something here speculatively — premature shared code becomes harder to evolve than two parallel copies.

### Today's surface

| Concept | Entry | Mirrors (Rust) |
|---|---|---|
| `<MarkdownInline />` (refs/tags fire `onRefClick`/`onTagClick`; external `[label](url)` links fire the optional `onLinkClick(href)` — when wired, the link is a keyboard-operable button (`role`/`tabindex`/Enter+Space); when omitted it's a plain inert `<span>`, no fake button. `((blk-X))` block-refs resolve their `text` from the `embeds` map when present — Roam-style inline reference, TODO/DONE marker prefixed — and fall back to the neutral handle chip when orphan; `!((blk-X))` embeds still render the single-line `↳ text`; `![alt](href)` **image** tokens render an inline `<img>` for image extensions (a local `assets/…` asset resolved to a `data:` URL via `readAssetDataUrl`; a remote src used directly, but only when `isSafeHttpUrl` passes so a `file:`/`data:`/`javascript:` href from synced/imported markdown is refused and shown as a chip instead of hit as an image src) and a clickable **file chip** for pdf / other kinds that fires `onLinkClick(href)` so the client opens it in the OS viewer; in the compact `inline` variant every asset renders as an inline-sized chip instead of a block image, issue #203) | `@outl/shared/markdown` | output of `outl_md::tokenize_owned` |
| `<EmbeddedSubtree nodes= embeds? depth? />` — read-only render of a `!((blk-X))` embed's source subtree as nested `↳` rows (TODO/DONE markers, nested refs/embeds via `embeds`), depth-capped at 4. Display-only, no onClick/edit/nav — the client wires the expansion under the carrying block | `@outl/shared/markdown` | mirror of `outl-tui`'s `emit_embedded_children` (`view/outline.rs`), `EMBED_MAX_DEPTH = 4` |
| `EmbedMap` (`Record<handle, ResolvedBlock>`) — resolved refs/embeds keyed by handle, consumed by `<MarkdownInline embeds= />` and `<EmbeddedSubtree embeds= />` | `@outl/shared/markdown` | reply shape of `resolveEmbeds` |
| `splitQuote`, `isQuote`, `QUOTE_PREFIX`, `stripQuoteFromTokens` | `@outl/shared/markdown` (re-exported) | `outl_actions::quote::{split_quote, is_quote, QUOTE_PREFIX}` |
| `<QuoteWrap />`, `isBlockQuoted` | `@outl/shared/markdown` | Wraps a quoted block's body in blockquote chrome (left border + faint tint), while each client keeps its outline bullet outside. Each client passes its theme tokens via `baseClass` + `chromeClass` props (Tailwind string literals for JIT discovery). |
| `isAssetLink(url)` — true when a `[label](url)` link points at a workspace asset (`assets/…`, `./assets/…`, `/assets/…`; any `://` scheme rejected). Every client's `handleLinkClick` routes an asset link to `openAsset` (OS default app) and everything else to `openExternalUrl` | `@outl/shared/links` | mirror of `outl_md::asset::is_asset_link` |
| `isImagePath(href)` → `boolean` (extension classifier — strips `?`/`#`, matches the last `.`-segment; drives whether `<MarkdownInline />`'s `image` token renders an `<img>` or a click-to-open file chip) + `assetFileName(href)` (last non-empty path segment, for the chip label) + `isSafeHttpUrl(href)` → `boolean` (`http`/`https` only; gates whether a remote image href is loaded directly as `<img src>` or refused to a chip, since the href comes from synced/imported markdown) | `@outl/shared/links` | `isImagePath` mirrors `IMAGE_EXTENSIONS` in `crates/outl-md/src/wikilink.rs` (`is_image_target`) — keep the extension list in sync |
| `formatNextFire(iso, now?)` → `"now"` / `"in 20min"` / `"in 3h"` / `"tomorrow 09:00"` / `"Dec 15, 10:00"`, and `groupReminders(list, now?)` → Today / Tomorrow / This week / Later / Done (empty buckets dropped). Both GUI clients render the same column and the same section headers; two implementations drift on the edge cases (exactly 60 minutes, midnight rollover, a finished rule) long before anyone notices | `@outl/shared/api/commands` | No Rust mirror — presentation only. **The instants themselves come from `outl_actions::reminders::next_fire_at`**, which is the single owner of *when* a reminder fires. Never re-derive a schedule in TS |
| `Reminder` / `ReminderSettings` DTOs. Times are ISO-8601 **local** strings with no zone suffix, deliberately: the backend resolved them in the user's configured timezone (`outl_actions::clock`), and re-deriving a local time from an epoch in JS reintroduces exactly the bug that module exists to fix | `@outl/shared/api/types` | `outl_tauri_shared::commands::reminders::{ReminderDto, ReminderSettingsDto}` |
| `<BlockProperties />` — a block's `key:: value` chips under its text, editable in place when `onCommit` is wired (click a chip → `<input>` → Enter/blur commits, Esc reverts, empty clears). Both clients rendered **nothing** here before, so a `remind::` written by a chord left the block looking untouched — invisible and uneditable from the outline. The inline editor is shared rather than per-client because an `<input>` is an input whether a keyboard or the iOS one drives it; only the theme classes differ. `propertyChips(props)` is the pure policy underneath: which keys get a glyph (`⏰ remind`, `▶ auto-run`, `📋 template`), which are the user's own (`priority: high`, rendered verbatim — interpreting it isn't ours), and which are outl's bookkeeping (`id`, `from-template` — hidden). `remindRule(props)` is the "does this block nag me" shortcut. `isInternalKey(key)` exports the bookkeeping-key predicate on its own, so property editors (the mobile sheet's key chips) hide the same keys the chip row hides instead of keeping a copy of the list | `@outl/shared/markdown` | glyph table mirrored by `outl-tui`'s `property_glyph` (`view/outline.rs`) — a const can't cross the Rust/TS boundary, so the two are edited together |
| `looksLikeOutline` | `@outl/shared/paste` | `outl_actions::paste::looks_like_outline` |
| `hasMultipleParagraphs` | `@outl/shared/paste` | mirror of `split_paragraphs(...).length > 1` in `outl_actions::paste` — gate that decides whether plain text needs the structured backend path |
| `htmlToOutlMarkdown` | `@outl/shared/paste` | Rich-clipboard `text/html` → outl markdown via **Turndown**, configured for the outl dialect (`*italic*` not `_italic_`, `**bold**`, `- ` bullets collapsed to 2-space nesting, `~~strike~~`, inline `<img alt>` → its alt text so Slack `:emoji:` survives). No Rust mirror — HTML only reaches the GUI webview clients; the resulting markdown then rides the same `paste_markdown_at` backend path as any paste |
| `choosePasteRoute(html, plain)` → `PasteRoute` | `@outl/shared/paste` | The one owner of the paste-with-formatting routing decision (`rich` = HTML converted to markdown; `structured` = plain outline / multi-paragraph; `native` = trivial, let the browser splice). Desktop `handlePaste` and mobile `onPaste` both call it, so the gate can't drift between clients — it used to be duplicated inline in each handler |
| `utf16OffsetToCharOffset` | `@outl/shared/paste` | (runtime gap — UTF-16 ↔ codepoint, no Rust mirror) |
| `detectRefContext`, `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion` | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::detect_trigger` |
| `detectSlashContext` / `applySlashContext` (+ `SlashContext`) — block-initial `/command` trigger + token removal on accept, powering the desktop's inline slash menu (Notion-style); mirrors the TUI `/` slash overlay but inline in a block | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::slash_candidates` (same command universe, different surface) |
| `autoPairBracket` (single `(`/`[`/`{` auto-pair + closer step-over; `autoDeletePair` also collapses empty `()`/`[]`/`{}`) | `@outl/shared/autocomplete` | `outl_tui::input::insert` (`insert_pair`) + `EditBuffer::delete_pair_back` |
| `<ParseWarningsBanner />` + `@outl/shared/warnings/styles` CSS | `@outl/shared/warnings` | TUI `view::warnings_banner` (visual parity, neutral chrome). Clients **must** `@import "@outl/shared/warnings/styles"` from their root stylesheet — without it the banner renders with unstyled neutral classes and looks invisible against the page. |
| `ParseWarning` / `ParseWarningKind` (DTO of `PageView.warnings`) | `@outl/shared/api/types` | `outl_md::ParseWarning` / `ParseWarningKind` |
| `<PageAheadOfLogBanner info= client= />` + `aheadOfLogNotice(info, client)` + `RECONCILE_COMMAND`. Rendered above the outline when `PageView.md_ahead_of_log` is present: the page has stopped syncing because its `.md` holds lines the op log never recorded, so outl refuses to overwrite the file (root `CLAUDE.md` invariant 8). `aheadOfLogNotice` is the pure copy owner and is what's unit-tested — the wording is the deliverable here, not the markup, and it must say the same thing on both clients. `client` changes exactly one sentence: the desktop user gets the command to run in the workspace folder, the mobile user gets "open this workspace on your computer", because **iOS ships no `outl` binary** and a button that can't work would be worse than saying so. Not dismissable — the condition doesn't clear on its own. Each client keeps the notice across mutation replies (which never carry it) and drops it on the first reply with `md_ahead_of_log_checked` and no notice, so the banner can't outlive the reconcile that fixed the page | `@outl/shared/warnings` (+ `@outl/shared/warnings/styles` CSS) | `outl_tauri_shared::state::MdAheadOfLog`, produced by `helpers::reproject_stale_md`; the recovery is `outl reconcile --ahead-of-log` |
| `MdAheadOfLog` (`path`, `lines`, `sample`) — DTO of `PageView.md_ahead_of_log`; `PageView.md_ahead_of_log_checked` rides alongside it and says whether this reply ran the check (only the open commands do), i.e. whether an absent notice means "healthy" or "unknown" | `@outl/shared/api/types` | `outl_tauri_shared::state::MdAheadOfLog` |
| `<PairingQR ticket=… />` (renders a pairing ticket as a scannable QR; owns its own async encoding via `ticketToSvg`, **no invoke inside** — host passes the ticket from `peerPairHost()`) + `<PeerList peers=… statusByNodeId? onRemove? />` (pure list of paired devices with online/offline/unknown status dot + optional remove button; **all data + callbacks via props, no invoke**) + `ticketToSvg` (pure ticket → SVG string, wraps the `qrcode` npm dep) + `peersOnline(statuses)` (pure: `true` when any peer has `online === true`; accepts the `PeerStatusDto[]` from `peerStatus()` or the desktop's `Map<node_id, …>`; both clients derive the sync dot from it identically) | `@outl/shared/peers` (+ `@outl/shared/peers/styles` CSS baseline) | the `outl_peer_*` commands in each client's `commands/peers.rs` (wrappers in `@outl/shared/api/commands`; `outl_sync_iroh::PeerEntry`/`PeerStatus`) |
| `PeerDto` (`node_id`, `alias`, `added_at`) / `PeerStatusDto` (`node_id`, `alias`, `online`, `rtt_ms`) | `@outl/shared/api/types` | Rust `PeerDto` / `PeerStatusDto` in both clients' `commands/peers.rs` |
| `createSyncProgress()` → `SyncProgressState { current, feed, clear }` (subscribes to the `sync-progress` Tauri event, resolves `received-ops` block ids to page/journal slugs via `resolvePageLabels`) + `<SyncProgressView current= feed= peers= />` (pure: phase pill + snapshot-% bar / live ops count + "device → page" activity feed; **no invoke inside**, host wires `createSyncProgress()` and passes the signals down) + `SyncFeedEntry` / `SyncProgressState` types. Both `DevicesSheet` (mobile) and `SyncPanel` (desktop) render this one implementation on the pairing screen | `@outl/shared/peers` (+ `@outl/shared/peers/styles` CSS baseline) | `outl_actions::SyncProgress`, bridged to the `sync-progress` event by `outl-tauri-shared::iroh_sync::start_with_reload_bridge` |
| `SyncProgress` — tagged union by `phase` (`connecting` / `snapshot` / `asset` / `received-ops` / `pushed-ops` / `synced` / `failed`), every variant carries `peer` (short node id); `snapshot` + `asset` also carry byte `received` / `total` (progress bar) | `@outl/shared/api/types` | `outl_actions::SyncProgress` (`#[serde(tag = "phase", rename_all = "kebab-case")]`) |
| First-run onboarding copy (`STORAGE_STEP`, `SYNC_STEP`, `FINISH_CTA`) — plain `as const` data, **no invoke / no JSX**; the only piece of onboarding that's identical between clients (the honest, no-account "where do your notes live" + "sync is peer-to-peer, one device is fine" wording). The chrome is client-specific (mobile: full-screen bottom-sheet-styled `Onboarding.tsx` + haptics; desktop: `Onboarding.tsx` wrapping `<WorkspacePicker />` + `<SyncPanel />`). | `@outl/shared/onboarding` | no Rust mirror — UI copy. The storage facts it tracks live in `outl-mobile/src-tauri/workspace_picker.rs` / `outl-desktop` workspace commands |
| DTOs (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink` — incl. `ancestors: BacklinkCrumb[]`, the citing block's root-first ancestor breadcrumb, page root excluded, empty at root level — `InlineToken`, `PageView` — incl. `backlinks_order: BacklinksOrder`, `CreateBlockReply`, `WorkspaceSummary`, …) | `@outl/shared/api/types` | the corresponding `serde`-serialised Rust structs |
| `BacklinkCrumb { id, text }` — one breadcrumb entry (plain text, no TODO/DONE prefix); same shape as `FocusCrumb` | `@outl/shared/api/types` | `outl_actions::BacklinkCrumb` |
| `ResolvedBlock { handle, text, page_slug, status, children }` — a block resolved from its ref handle; reply value of `resolveEmbeds`. `((…))` refs render `text`; `!((…))` embeds render `text` + the `children` subtree | `@outl/shared/api/types` | Rust `EmbedContent` |
| `BacklinksOrder` (`"newest"` \| `"oldest"`) | `@outl/shared/api/types` | `outl_config::BacklinksOrder` |
| `PageBacklinks` (`backlinks` + `backlinks_order`) — reply of `pageBacklinks(slug)` / `setBacklinksOrder(...)`; backlinks are fetched lazily, decoupled from `PageView`, so the O(blocks) scan never blocks first paint | `@outl/shared/api/types` | `outl_tauri_shared::state::BacklinksReply` |
| Plugin DTOs (`PluginCommand`, `PluginToolbarButton`, `PluginRunReply`, `PluginSyncHooksReply`, `PluginTransformer`, `PluginTransformResult`) + wrappers (`pluginList`, `pluginRun`, `pluginSyncHooks`, `pluginToolbar`, `pluginTransformers`, `pluginTransform`) — both clients register the identical `plugin_*` commands (thin shims over `PluginService`), so the wire shapes + wrappers live here once. The desktop-only chord surface (`PluginKeybinding` / `pluginKeybindings`) stays in `outl-desktop/src/lib/api.ts` (mobile has no keybindings) | `@outl/shared/api/types` + `@outl/shared/api/commands` | each client's `src-tauri/src/commands/plugin.rs` DTOs |
| Content-transformer registry + cache (`loadTransformers`, `transformerFor(lang)` → `PluginTransformer \| null`, `runTransform(blockId, transformer, body)` cached by `(blockId, body)`) — the `lang → transformer` Solid signal both clients load once per workspace open, plus the per-fence result cache (a failed transform drops its key so a later render retries; a reload clears the cache since results may now differ) | `@outl/shared/plugins/transformer-registry` | no Rust mirror — client lifecycle glue over `plugin_transformers` / `plugin_transform`, identical on both clients |
| Mobile keyboard-toolbar **logic** — `ToolbarAction` catalog + `TOOLBAR_META` (label + symbol/text style) + `DEFAULT_ORDER` + `PINNED_FIRST`/`PINNED_LAST`, plus the MFU ordering (`orderedMiddleActions(counts)` pure; `recordToStore`/`readCountsFromStore`/`orderedMiddleFromStore` over `localStorage` under `outl.toolbar.mfu.v1`). The action string ids are the wire contract the iOS native bar ships via `window.__outlToolbar(action)`; **kept byte-identical to `OutlKit/Toolbar/ToolbarAction.swift`** (rename on both sides in one commit). Rendering (icons, capsule, keyboard docking) is client chrome, NOT here | `@outl/shared/toolbar` | port of `OutlKit/Toolbar/{ToolbarAction,ToolbarMFU}.swift` (the iOS native bar keeps its Swift copy because it renders before the webview) |
| `rawTextWithTodo(block)` — wire-format text with the task prefix (`TODO ` / `DOING ` / `DONE `) reattached, what every client's editor shows so the user can erase / type the prefix | `@outl/shared/outline` | mirror of `outl_actions::split_todo` in reverse (keep in sync with `TASK_PREFIXES`) |
| `cycleTodo(raw)` — none → `TODO ` → `DOING ` → `DONE ` → none on wire-format text, quote prefix peeled and re-emitted in canonical order (`TODO > body`, never `TODO > TODO body`). The **editing** path: a client mid-edit holds the text in a textarea draft seeded once, so routing the toggle through the backend updated the outline behind a stale draft and the new state only appeared on leaving Insert. Clients splice this into the draft instead, the way the TUI mutates its `EditBuffer`. **The stop list (`TASK_PREFIXES`) must match `outl_actions::todo` exactly** — this function exists to skip the round trip, so a cycle that disagrees shows the user a state the op log never receives | `@outl/shared/outline` | mirror of `outl_actions::todo::cycle_todo` (same pair `rawTextWithTodo` tracks) |
| Outline walks — `findBlock`, `flattenNodes` (DFS preorder, returns **`BlockNode`s**), `countDescendants`, plus the id-returning selection walks: `flattenVisible` (skips collapsed subtrees), `flattenAll` (every id, `zR`), `flattenParents` (`zM` fold-all targets — mirror of outl-tui's `collect_collapse_candidates`), `nextVisibleId` / `previousVisibleId` (vim `j`/`k`; previous returns `null` at the top, never the current block), `visualRangeIds` / `visualRangeSet` (memoise the Set at the parent — per-row predicates are O(N²)) / `isInVisualRange` | `@outl/shared/outline` | `outl-tui`'s outline walks (`collect_collapse_candidates` for `flattenParents`); pure functions over `BlockNode[]`, no invoke |
| `embedOnlyHandle(tokens)` → `string \| null` — the embed handle when a block's tokens are a single `!((blk-X))` embed (whitespace-only `plain` tokens ignored); `null` for mixed prose, a bare `((…))` ref, or two embeds. Gate for whether to expand `<EmbeddedSubtree />` under a block | `@outl/shared/outline` | mirror of `outl-tui`'s `embed_only_handle` (`view/outline.rs`) |
| `collectBlockRefHandles(outline)` → `string[]` — distinct `blockref` + `embed` handles across an outline (DFS on `children`, descends `bold`/`italic`/`strike` `inner`), de-duped by first appearance. Lets a client batch-resolve every handle in one `resolveEmbeds` call before render | `@outl/shared/outline` | no Rust mirror — client-side pre-resolve pass |
| `sameCrumbTrail(a, b)` — do two ancestor trails name the same chain of blocks, compared by id (all-or-nothing, not a shared-prefix count)? Generic over `{ id }`, so it works for both `BacklinkCrumb` and `FocusCrumb`. Drives the backlinks panel's breadcrumb collapse: consecutive references inside the same branch render the trail once | `@outl/shared/outline` | mirrored by `outl-tui`'s local `same_trail` in `view/backlinks.rs` (no upstream Rust owner — each Rust client keeps its own copy since the comparison is render-local) |
| `focusSubtree(blocks, blockId)` → `FocusView { root, breadcrumb } \| null` (+ `FocusCrumb { id, text }`) — zoom/focus: the subtree to render as the new root plus the ancestor breadcrumb (page-top first, parent last). `null` = stale zoom target (block deleted/moved) → caller drops the zoom. Zoom is **local view state**, never a Tauri round-trip (the client already holds the whole `outline`) | `@outl/shared/outline` | no Rust mirror — zoom is frontend-only view state |
| Journal slug + calendar math — `parseJournalSlug` / `formatJournalSlug` / `journalSlugToDate` (local-time parse; `new Date("YYYY-MM-DD")` is midnight UTC and renders the previous day in negative-offset timezones), `daysInMonth`, `MONTH_NAMES`, `DAY_LABELS` (Sunday-first, mobile sheet) / `DAY_LABELS_MONDAY_FIRST` (TUI-style, desktop sidebar), `mondayIndex`, `prevMonth` / `nextMonth` (pure year-rollover). The calendar **chrome** stays per-client — only the math/parsing is shared. `monthIndex` is 0-based everywhere (JS `Date` convention) | `@outl/shared/journal` | the `YYYY-MM-DD` journal slug contract (`outl_actions` date slugs); no Rust mirror for the grid math |
| `refReplacement(page, opts?)` — the page name spliced into `[[…]]` when a ref suggestion is accepted: journals insert their ISO slug, everything else (and every `@` mention) inserts the **title** (bug #88 was the chip strip writing the slug) | `@outl/shared/autocomplete` | no Rust mirror — pairs with `applySuggestion` |
| `invoke<T>()` wrappers (navigation: `listPages`, `searchPages`, `searchPersons`, `searchEmojis` → `EmojiHit[]` (powers the `:shortcode:` autocomplete in every client; backed by `outl_md::emoji::search` so TUI / mobile / desktop rank identically), `searchBlocks` → `BlockHit[]` (powers the `((…))` block-ref autocomplete; backed by `outl_md::WorkspaceIndex::search_block_text`; caller inserts each hit's `handle` wrapped in `((…))`, never the display `text`), `openTodayJournal`, `openJournalFor`, `openPageBySlug`, `openRef`, `previousDay`, `nextDay`, `todaySlug`, `dateTitle`, `resolveRef`, `workspaceStats`; mutation: `createBlock` → `CreateBlockReply` (returns `{ view, new_id }` so the client puts the new block straight into edit mode without diffing the outline), `splitBlock(pageId, id, charOffset) → CreateBlockReply` (splits a block at the caret — the text up to `charOffset` stays, the rest moves into a new sibling below, returned as `new_id`; `charOffset` is a codepoint offset, convert the textarea's UTF-16 `selectionStart` with `utf16OffsetToCharOffset` first; mirrors `outl_actions::split_block`, backs the Enter-mid-text gesture on desktop + mobile, issue #184), `editBlock`, `toggleTodo`, `deleteBlock`, `indentBlock`, `outdentBlock`, `moveBlockUp`, `moveBlockDown`, `reloadWorkspace`, `pasteMarkdown`, `pastePlain(pageId, blockId, caret, text)` (invokes `paste_plain_at` — paste without formatting: raw text as a single block, no normalisation or paragraph splitting), `copyMarkdown` (serialises a block selection + subtrees as clean outl markdown for the OS clipboard — the copy-out inverse of `pasteMarkdown`), `setBlockCollapsed`, `deletePage(slug) → Promise<PageView>` (delegates to the shared `delete_page` command; returns today's-journal `PageView` so every caller navigates away from the deleted slug identically — desktop hover `×`, desktop `DeletePage` action handler, and mobile long-press all call this one wrapper), `pageBacklinks(slug) → Promise<PageBacklinks>` (the **lazy** backlinks fetch every client fires after the outline paints — backlinks moved off `PageView` because `backlinks_for_page` is an O(blocks) scan that blocked the first journal paint; `PageView.backlinks` now always comes back empty, mirroring the TUI's lazy/cached panel), `setBacklinksOrder(order, slug) → Promise<PageBacklinks>` (delegates to `set_backlinks_order`; persists `[display] backlinks_order` and returns the re-sorted `PageBacklinks` — the desktop `InlineBacklinks` and mobile `BacklinksSection` header buttons both call this one wrapper, issue #142); execution: `runCodeBlock` → `RunCodeBlockReply` (refreshed `PageView` + stdout/stderr/exit so the caller swaps the outline in one round-trip); peers/pairing: `peerList` → `PeerDto[]`, `peerRemove(id)` → `bool` (prefix match), `peerStatus` → `PeerStatusDto[]` (async iroh probe), `peerPairHost(alias?)` → `string` (ticket; completion surfaces via the backend `peer-paired` event — desktop's Rust command is being aligned to the mobile ticket-return shape), `peerPairJoin(ticket, alias?)` → `PeerDto`, `syncNow()` → `void` (force an immediate iroh sync pass against every peer — pull-to-refresh / Refresh; no-op when iroh isn't wired), `resolvePageLabels(nodeIds) → Promise<string[]>` (batch-resolves block ids to their distinct page/journal slugs; the `createSyncProgress` feed's only round-trip, best-effort — an id not yet materialized on this device is skipped); external links: `openExternalUrl(href)` (opens `http(s)`/`mailto` in the system browser via `tauri-plugin-opener`; rejects other schemes — the host must register the opener plugin + grant `opener:allow-open-url`); assets: `openAsset(url) → Promise<void>` (opens an `assets/…` uploaded file in the OS default app via the `open_asset` command — outl does not render it; rejects when the asset hasn't synced to this device yet), `readAssetDataUrl(url) → Promise<string>` (resolves an `assets/…` link to a `data:<mime>;base64,…` URL the webview loads inline via the `read_asset_data_url` command — the image-render path; size-capped backend-side, rejects when the asset hasn't synced yet), `attachAsset(sourcePath, pageId, afterBlockId?) → Promise<PageView>` (imports a file via the `attach_asset` command — content-addressed copy into `<root>/assets/`, size-capped — and attaches its markdown link as a new block after `afterBlockId` or at the page end; both clients drive it from a `@tauri-apps/plugin-dialog` file picker), and `importAssetFile(sourcePath) → Promise<ImportedAsset>` (imports via `import_asset_file` — same content-addressed copy, but creates **no** block; returns `{ rel_path, display_name, is_image, markdown }` so the caller splices `markdown` straight into the drop target — the drag-and-drop path, see below)) | `@outl/shared/api/commands` | the matching Tauri command in each client's `src-tauri/src/lib.rs` (`openExternalUrl` wraps the `@tauri-apps/plugin-opener` JS API, not a custom command) |
| `ImportedAsset` (`rel_path`, `display_name`, `is_image`, `markdown`) — reply of `importAssetFile`, mirror of `outl_actions::ImportedAsset` | `@outl/shared/api/commands` | `outl_actions::ImportedAsset` |
| `installFileDrop(handlers)` → `Promise<UnlistenFn>` — wires the Tauri webview's `onDragDropEvent` (`onEnter`/`onOver`/`onLeave` optional, `onDrop(paths, blockId)` required); resolves the hit-tested block id under the drop from the raw physical-pixel position. `physicalToCss(position, dpr)` (HiDPI physical→CSS), `blockIdFromElement(el)` / `blockIdAtPhysical(position)` (`.closest("[data-block-id]")` hit-test), `joinAssetMarkdowns(markdowns)` (space-join, drop empties), `appendMarkdownToBlock(existing, markdown)` (space-separated append, no leading space on an empty block) are the pure helpers underneath, unit-tested; `installFileDrop` itself needs a real webview. Desktop and mobile both wire it identically so the drop geometry can't drift between clients | `@outl/shared/drag-drop` | no Rust mirror — OS drag-drop only reaches the GUI webview clients; the import itself rides `importAssetFile` |

## What does NOT enter the library

- **Chrome.** `<Sidebar />`, `<Picker />`, `<BacklinksPanel />`, `<BlockRow />`, app shells — they diverge between mobile (single-pane, touch) and desktop (3-pane, mouse + vim mode).
- **Stateful stores.**
  Each client's Solid `createStore()` carries client-specific shape (mobile has swipe state, desktop has panel collapse state).
- **Keybindings.**
  Cmd-based on desktop, gesture-based on mobile.
- **Client-specific Tauri commands.**
  `pick_workspace_dir` belongs to `outl-desktop`; the iCloud peer-files watcher and gestures glue belong to `outl-mobile`.
  Wrap those in the client's own `lib/api.ts`.
  (`run_code_block` *used* to live here too; mobile picked up the same command in v0.6.x — long-press → "Run code" — so the wrapper is now in `@outl/shared/api/commands`.
  Desktop's `lib/api.ts` re-exports it for backward-compatible imports.)
- **Tailwind config.**
  Each client has its own theme; could be shared later if the palettes converge.
  Low priority.

## Theming note

The `<MarkdownInline />` component currently uses iOS-themed CSS custom properties (`--color-ios-accent`, `--color-iosd-*`).
The mobile client defines them in its stylesheet; **the desktop client must mirror the same token names** until we refactor to neutral `--color-outl-*` tokens.
If desktop's palette diverges first, introduce the abstraction in this library and have each client map its theme to the neutral tokens.

## Adding a new piece

1. **Search first.**
   Before writing a helper in any client `lib/`, `rg` here and in `outl-mobile/src/lib/` for a comparable name or symbol.
2. **If the other client has it locally**, promote in the same PR (move to `src/<area>/`, update both clients' imports, delete the local copy).
3. **If it's a brand-new concept that only one client needs today**, write it in the client.
   When the second client wants it, promote in the move PR.
4. **Update the table above** when promoting.

## Running tests

```bash
bun install                        # at repo root, hoists deps via workspaces
bun --filter @outl/shared test     # just this library
bun --filter outl-mobile test      # mobile (consumes this library)
```

## When you're done editing

1. `bunx tsc --noEmit` from this crate (type check)
2. `bun --filter @outl/shared test` (Vitest)
3. `bun --filter outl-mobile test` (paridade — mobile consume idêntico)
4. If you changed the public surface (a new file in `src/`, a new export in `package.json` `exports`), update:
   - This file's "Today's surface" table
   - Each consuming client's `CLAUDE.md` if the contract is new
   - Root `CLAUDE.md` "Shared primitives catalog" (frontend section)
