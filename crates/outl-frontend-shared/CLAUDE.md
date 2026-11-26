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
- What a search box lists and in what order (`@outl/shared/palette`: pages, tasks, blocks, actions, the `>` prefix, per-section limits).
  Chord-aware action rows stay on the desktop, because the phone has no chords.

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
    │   ├── commands.ts     # invoke<T>() wrappers + re-exports of the four below
    │   ├── commands-peers.ts      # peer pairing / iroh transport
    │   ├── commands-plugins.ts    # plugin host + marketplace
    │   ├── commands-reminders.ts  # `remind::` + its two display helpers
    │   ├── paste.ts        # PasteReply + pasteMarkdown / pastePlain / pasteBlockAfter (DTO beside its wrappers)
    │   ├── embeds.ts       # ResolvedBlock + PageEmbedContent + resolveEmbeds / resolvePageEmbeds (same arrangement)
    │   ├── page-merge.ts   # rename / merge DTOs + renamePage / mergePages (own file: commands.ts and types.ts are at the ratchet)
    │   └── backlinks.ts    # pageBacklinks / setBacklinksOrder + the cross-page mutation family (editBacklinkBlock / backlinkRowOp / createBacklinkSibling) + `unfoldAncestors` (the breadcrumb arrival's fold opener) — the panel's commands, same move for the same reason. In ts_parser::MIRROR_FILES since it gained BacklinkRowOp / BacklinkSide
    ├── backlink-edit/
    │   ├── index.ts        # backlinkEditTarget / backlinkCommitPlan / backlinkCreatedElsewhere — what a gesture on a backlink row targets, what its reply may touch, and what to say when a create lands elsewhere
    │   └── backlink-edit.test.ts
    ├── pages/
    │   ├── rename-merge.ts # renameOrMerge / mergeInto flow + the rename and merge wording
    │   └── rename-merge.test.ts
    ├── boot/
    │   ├── index.ts        # bootMark / waitWorkspaceReady / onPagePreview / the first-frame style cache
    │   ├── PreviewOutline.tsx  # read-only outline for the pre-workspace preview; no interaction, by design
    │   └── boot.test.ts
    ├── capture/
    │   └── index.ts        # installForegroundRefresh / drainOnce — the inbox drain both GUIs run
    ├── confirm/
    │   ├── index.ts        # createConfirmQueue — "are you sure?" as a promise. The bridge from an awaited boolean to a signal a dialog renders; the dialog itself stays per client (a modal card on the desktop, an iOS alert on the phone). The request payload is a type parameter so each client's chrome fields stay its own
    │   └── confirm.test.ts
    ├── warnings/
    │   ├── ParseWarningsBanner.tsx
    │   ├── PageAheadOfLogBanner.tsx
    │   ├── ahead-of-log.ts        # aheadOfLogNotice — the copy, one owner
    │   ├── ahead-of-log.test.ts
    │   ├── styles.css
    │   └── index.ts
    ├── navigation/
    │   ├── page-key.ts     # createPageKey — the one "did the page change?" memo
    │   ├── page-key.test.ts
    │   └── page-key.parity.test.ts  # source-read over BOTH clients; declared exceptions only
    ├── markdown/
    │   ├── CrumbTrail.tsx      # a backlink's ancestor breadcrumb, rendered; inert unless the client passes onRefClick/onTagClick
    │   ├── MarkdownInline.tsx
    │   ├── MarkdownTable.tsx   # a block's `table` as an HTML <table>
    │   ├── PageEmbed.tsx       # PageEmbedMap + the `![[page]]` title/literal line (its geometry rule is in the file)
    │   └── index.ts
    ├── caret-nav/
    │   ├── index.ts        # caretExit (does this arrow leave the block?) + resolveCaretExit (which block, and where in it)
    │   └── caret-nav.test.ts
    ├── paste/
    │   ├── index.ts        # looksLikeOutline/Table/Tabular, choosePasteRoute, utf16/char offsets, pasteEditorUpdate
    │   ├── table.ts        # clipboard <table> → markdown table (Turndown ships no table rule)
    │   └── paste.test.ts
    ├── autocomplete/
    │   ├── index.ts        # autoClosePair, autoPairBracket, autoDeletePair, insertPair, insertText, detectRefContext, applySuggestion
    │   ├── emoji.ts        # the `:shortcode:` trigger, split out at the 600-line ratchet; re-exported from index.ts
    │   ├── slash.ts        # the `/command` trigger, same split, same re-export
    │   └── autocomplete.test.ts
    ├── onboarding/
    │   ├── index.ts        # shared copy (STORAGE_STEP, SYNC_STEP, FINISH_CTA, CRASH_REPORTS_SETTING, CRASH_REPORTS_NOTICE) — plain data, no invoke
    │   └── onboarding.test.ts
    ├── mock/
    │   ├── index.ts        # installMockBackend + wantsMock: `?mock=1` runs a client in a plain browser (dev only)
    │   ├── backend.ts      # command name → reply, from an in-memory fixture workspace
    │   ├── workspace.ts    # the fixture pages; not a CRDT, no op log
    │   ├── tokenize.ts     # tiny stand-in for outl_md::tokenize_owned (refs, tags, bold, code)
    │   └── presets.json    # every outl_theme preset; outl-theme's presets_fixture test keeps it current
    ├── journal/
    │   ├── index.ts        # parseJournalSlug, formatJournalSlug, journalSlugToDate, daysInMonth, MONTH_NAMES, DAY_LABELS*, mondayIndex, prev/nextMonth
    │   ├── day-clock.ts    # createDayClock + shouldWakeToToday — which day it is, for a client open past midnight (re-exported from index.ts)
    │   ├── day-clock.test.ts
    │   └── journal.test.ts
    ├── outline/
    │   ├── index.ts        # rawTextWithTodo, findBlock, flattenNodes, countDescendants + id walks (flattenVisible/All/Parents, next/previousVisibleId, visualRange*)
    │   ├── move-gate.ts    # createMoveGate — one block move in flight at a time; a held chord drops the repeats that arrive while one is pending
    │   ├── range-move.ts   # rangeMoveTargets — which blocks a Visual range move acts on (the selection's roots, in walk order)
    │   ├── reveal.ts       # landingTarget + needsUnfold — where an arrival from a backlink crumb lands and whether a fold is in the way (re-exported from index.ts)
    │   ├── late-section.ts # lateSectionVisible — when a section fed by a post-first-paint reply is on screen (owns the rule; re-exported from index.ts)
    │   ├── page-embed.ts   # pageEmbedOnlyName + collectPageEmbedNames — the `![[page]]` twins of embedOnlyHandle / collectBlockRefHandles (re-exported from index.ts)
    │   └── outline.test.ts
    ├── drag-drop/
    │   ├── index.ts          # installFileDrop, physicalToCss, blockIdFromElement/AtPhysical, joinAssetMarkdowns, appendMarkdownToBlock
    │   └── drag-drop.test.ts
    ├── plugins/
    │   └── transformer-registry.ts  # content-transformer registry + (blockId, body) result cache
    ├── palette/
    │   ├── index.ts        # paletteResults, sectionOf, DEFAULT_LIMITS, COMMAND_PREFIX, BLOCK_QUERY_MIN
    │   └── palette.test.ts
    ├── reminders/
    │   ├── index.ts        # EMPTY_REMINDER_NOTICE + reminderHasText — the empty-block row's copy
    │   └── reminders.test.ts
    ├── tasks/
    │   ├── index.ts        # groupTasksByPage — one section per page for a task list
    │   └── tasks.test.ts
    ├── toolbar/
    │   ├── actions.ts       # ToolbarAction catalog + TOOLBAR_META + DEFAULT_ORDER + PINNED_FIRST/LAST
    │   ├── mfu.ts           # most-frequently-used ordering (localStorage) — port of Swift ToolbarMFU
    │   ├── mfu.test.ts
    │   ├── lock.ts          # the user's opt-out from MFU — port of Swift ToolbarLock
    │   ├── lock.test.ts
    │   ├── keys.test.ts     # pins both storage-key literals (cross-language contract)
    │   ├── storage.ts       # internal: defensive localStorage wrapper shared by mfu + lock
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

### Today's surface lives in `docs/`

The catalog of what already exists — every entry, where it is imported from, and the Rust source it mirrors — is three pages under `docs/`, because it is reference and this file is loaded on every session in the crate:

| Page | What it lists |
|---|---|
| [`docs/frontend-shared-views.md`](../../docs/frontend-shared-views.md) | Components and the view pieces both clients render |
| [`docs/frontend-shared-wire.md`](../../docs/frontend-shared-wire.md) | The DTOs the Rust backend serialises, and the typed `invoke()` wrappers |
| [`docs/frontend-shared-logic.md`](../../docs/frontend-shared-logic.md) | Pure helpers: paste, autocomplete, outline walks, journal math, caret navigation |

**Search all three before writing a helper:** `grep -n 'symbol' docs/frontend-shared-*.md`.


## The transclusion rail, and where the TUI stops being the model

`<EmbeddedSubtree />` used to draw a `↳` on every row, copied from the
TUI. On a thirteen-row page that read as a wall of arrows, and the
glyph was answering the wrong question: it says *"this row is nested"*,
which the indentation already says, while nothing said *"this came from
another page"* — the one fact a reader cannot infer.

So the GUI clients now draw **one rail** for the whole region instead,
and the shared promise with the TUI is the semantics (read-only,
depth-capped at 4, both embed kinds through one component), not the
glyph. The `↳` is still right in a terminal, which has no hairline to
draw and no second axis to spend.

Three constraints that are not preferences, each from `DESIGN.md`:

- **`accent` at 25%, never solid.** The selected block's 3px rail is
  solid `accent` ("The two odd numbers"). Same hue on purpose — both
  mean "outl is marking this region" — but a cursor is one row and a
  transclusion is many, so reusing the solid weight makes the page look
  permanently selected.
- **22px per level**, the product's own outline rhythm. This component
  stepped at `1rem` for months, so borrowed rows drifted out of phase
  with the page around them the deeper they went.
- **No fill, no card, no shadow.** `bg_elev` is the only elevation
  colour and belongs to things that float; an embed sits *in* the page.

**A marker glyph on the title was tried and reverted.** An `<svg>` is an
atomic inline box that carries its own height into the line box, and
that line swaps a round-trip after first paint inside a block the user
may be typing in. `a_page_embed_title_is_a_plain_inline_span` refuses
it; the weight and the hue carry the distinction for free.

## Props are read lazily, never destructured

Solid's reactivity rides the getter.
`function Foo({ view })` snapshots `view` at first render and never updates again — the component simply stops reacting, with no error, no warning and no failing test.
`props.view` is the only correct form, in this library and in every client component.

This is the one bug an extraction reliably introduces, because pulling JSX out of a big component is exactly when the temptation to destructure a fifteen-prop signature shows up.
`JournalChrome.tsx` (extracted from `outl-mobile`'s `Journal.tsx`) carries the rule in its own doc comment for that reason.

The same care applies to an optional prop's sentinel: `todaySlug: string | null` means `null`, and collapsing it to `""` at the call site destroys the distinction the receiving component tests on.

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

`<MarkdownInline />`, `<EmbeddedSubtree />` and `<PageEmbedTitle />` read only the canonical `--color-outl-*` custom properties (RFC 0022).
The legacy `--color-ios-*` / `--color-iosd-*` namespace they used to read is gone — grep for `--color-ios` in this crate's `src/` and expect nothing but historical comments.
`@outl/shared/theme::applyPaletteToRoot` is the single writer of `--color-outl-*`, driven by the `Palette` the backend's `get_theme` command resolves.
**Both GUI clients are wired.**
`SettingsModal.tsx`'s live preview reinstalls the draft pair through `installTheme`, so later OS appearance changes use the previewed values too.
Both clients' boot path also goes through `@outl/shared/theme::installTheme`, which resolves the full `[theme]` pair via `getThemeConfig()` and calls `applyPaletteToRoot` itself, on boot and on every `prefers-color-scheme` flip.
`installTheme` (plus `pickSide`) moved here from `outl-mobile/src/lib/theme.ts` as part of that wiring.
See `docs/theming.md` § "Light / dark pair and `mode`" for the client-by-client resolution and Settings preview/restore lifecycle.

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

## `api/commands.ts` and `api/types.ts` are split, their import paths are not

Both outgrew the 600-line ratchet, and both answered it by moving a
cohesive group into a sibling and re-exporting it, so
`@outl/shared/api/commands` and `@outl/shared/api/types` stay the single
import paths: `commands-{peers,plugins,reminders}.ts`, and
`{tokens,table,plugins,peers}.ts`.
`backlinks.ts` is the same move with no DTO to carry: two of its three
wrappers return the same `PageBacklinks` and the third is the only
mutation that acts on a row of it, so they travel together.
`paste.ts`, `page-merge.ts` and `embeds.ts` go one step further and keep
the DTO *beside its wrappers*, which is the right shape when the type
has no reader other than the command that returns it. Add a wrapper or DTO to whichever file
owns that area and re-export it — a second import path is the drift this
library exists to prevent.

**A split DTO file also goes in `ts_parser::MIRROR_FILES`**
(`outl-tauri-shared/tests/`): the pins read those files concatenated, so
a declaration in a file that list omits is invisible, never reported.

## When you're done editing

1. `bunx tsc --noEmit` from this crate (type check)
2. `bun --filter @outl/shared test` (Vitest)
3. `bun --filter outl-mobile test` (paridade — mobile consume idêntico)
4. If you changed the public surface (a new file in `src/`, a new export in `package.json` `exports`), update:
   - This file's "Today's surface" table
   - Each consuming client's `CLAUDE.md` if the contract is new
   - Root `CLAUDE.md` "Shared primitives catalog" (frontend section)
