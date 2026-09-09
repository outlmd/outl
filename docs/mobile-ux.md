# Mobile UX behaviour

How `outl-mobile` behaves, feature by feature.

Moved out of `crates/outl-mobile/CLAUDE.md`, which is loaded into the model's context on every session in that crate and had grown past the point where that is useful.
These sections are reference: you read one when you touch that feature, not every time you open the crate.
The rules a contributor must not break stayed in `CLAUDE.md`.

Platform plumbing has its own homes: [`docs/ios-platform.md`](ios-platform.md), [`docs/android-platform.md`](android-platform.md).
Anything visual — colours, tokens, spacing, components — is specified in [`DESIGN.md`](../DESIGN.md).

## Opening a ref that may not exist yet

`[[avelino/outl]]`, `[[2026-06-04]]`, `#code-review`, picker entries — every "tap a ref → see a page" path on the frontend goes through **one** Tauri command, `open_ref(target)`, which wraps `outl_actions::page::open_or_create_by_ref`.
The single decision tree (date → journal, else literal/slugified/title match → existing page, else create as page) lives in the shared crate so a frontend regex cannot drift from a backend parser the way it did before `open_ref` existed.

What used to be wrong: the frontend split the journal-vs-page
decision with `/^\d{4}-\d{2}-\d{2}$/` and routed to one of two
strict-validating commands (`open_journal_for` / `open_page_by_slug`).
`[[2026-13-01]]` matched the regex, hit `open_journal_for`, and
surfaced an `invalid date slug` toast — even though falling through
to "create a regular page" was clearly the right behaviour.

`open_page_by_slug` is kept for the picker (the picker already hands the command a clean slug from a known page).
`open_journal_for` stays for date-navigation commands (`previousDay` / `nextDay`) whose input is derived from controlled state, not from a user tap.
Every **ref-click** code path on the frontend (`handleRefClick`, `handleTagClick`) must call `openRef` so the decision tree is single-sourced.

`resolve_ref` survives for autocomplete previews ("this ref will
land on `<page>`") but is **not** the navigation entry point — for
that, always call `openRef`.

## Page switcher — long-press to delete

`PageSwitcher.tsx` renders each page as a row button; spreading `longPressHandlers(p)` arms a 500 ms sustained-touch detector (canceled if the finger moves more than 10 px).
On fire, `handleDelete(p)` runs `window.confirm(...)` → `deletePage(slug)` → navigates to the returned today's journal → refetches the list.
Journals are excluded — only regular pages can be deleted from the switcher.
The backend command is the shared `outl_tauri_shared::commands::page::delete_page` body — no mobile-specific logic.
`Action::DeletePage` carries a `g d` chord in the shared catalog (Normal mode), but mobile has no keyboard surface — long-press in the page switcher remains the only trigger on touch devices.

`BacklinksSection.tsx`'s `order`/`onToggleOrder` flips order via `setBacklinksOrder` (returns `PageBacklinks`); backlinks are lazy in `Journal.tsx` via `createResource(slug, pageBacklinks)` since `PageView.backlinks` is empty.

## Opening an external `[label](url)` link

Tapping an external link opens it in the system browser via **`tauri-plugin-opener`** (registered in `lib.rs`, capability `opener:allow-open-url` for `http(s)`/`mailto`).
`Journal.tsx`'s `handleLinkClick` calls the shared `openExternalUrl` — same as desktop, so the allow-list (`http(s)`/`mailto`; `file:`/`javascript:` rejected) lives in one place.
`<MarkdownInline />` gets `onLinkClick` threaded from `Journal.tsx` → `BlockRow` → the renderer.
An `assets/…` link routes instead (via `isAssetLink`) to `openAsset` — `open_asset` opens the file in the OS viewer.
The block long-press **Attach file** action picks a file (`@tauri-apps/plugin-dialog`) → `attachAsset` (shared `commands::asset`).
On iPad, dragging a file onto a block imports it the same way via the shared `installFileDrop` + `importAssetFile` (`@outl/shared/drag-drop`), best-effort — iPhone rarely delivers a webview drop, so long-press stays the only import path there.
`[[ref]]`/`#tag` taps still route through `openRef`; backlink rows stay inert.

## Blockquote chrome

A `"> "`-prefixed block gets a left border + ~5% tint, right-rounded, body full-colour (refs / bold / tags keep their palette).
The outline bullet and `<CollapseTriangle />` stay outside the quote chrome; a non-quoted block degrades to a plain flex container (byte-identical).
Detection is `splitQuote` + `stripQuoteFromTokens` (`@outl/shared/markdown`, mirror of `outl_actions::quote::split_quote`) so the `> ` isn't rendered twice; it composes with the task checkbox.
The checkbox has three filled states: DONE is a solid accent circle with a tick, **DOING is an accent ring with a small filled centre**, TODO is a neutral empty ring.
One tap walks one stop of `TODO → DOING → DONE → none`.
Toggling: `toggleQuote(id)` → `toggle_quote` → `outl_actions::block::toggle_quote` (no TS string surgery).
Convention (three-surface parity): [`docs/clients.md` → Blockquote convention](clients.md#blockquote-convention).

## Zoom / focus on a block

Tap a block's plain bullet dot to zoom in — it becomes the outline root (Roam/Workflowy focus); `← Back` + breadcrumb zoom out.
Local view state (`focusBlockId` in `Journal.tsx`), never a Tauri round-trip; the shared `focusSubtree` (`@outl/shared/outline`) does the subtree + breadcrumb.
Mobile owns only the touch chrome: the bullet tap moves mark-as-TODO to the long-press menu; checkbox + `<CollapseTriangle>` untouched.
Convention: [`docs/clients.md` → Zoom / focus on a block](clients.md#zoom--focus-on-a-block-roamworkflowy).

## Paste from external apps

The textarea in `BlockRow.tsx` intercepts paste (with formatting only — mobile has no `Cmd+Shift+V`).
Rich `text/html` converts via `htmlToOutlMarkdown` (`@outl/shared/paste`); plain text routes to `paste_markdown_at` when `looksLikeOutline` **or** `hasMultipleParagraphs`, splitting multi-paragraph into one block each (else native splice).

`create_block` has a **stale-anchor fallback**: if `after_id` is not in the tree (`NotInTree`), the block is appended at the end of the page instead of returning an error (mirrors the desktop fix).

The long-press context menu's "Copy" action calls `copy_markdown` (`commands/block.rs` → `outl_actions::copy_markdown`), serialising the block and its full subtree as clean outl markdown to the iOS clipboard.

## Keyboard accessory bar (Android web bar / iOS native bar)

The keyboard toolbar + suggester strip have two renderings.
iOS is native (`OutlToolbarView` swizzled onto `WKContentView`), untouched.
Android is web: `KeyboardAccessory.tsx` → `<SuggesterStrip />` + `<KeyboardToolbar />`, gated in `Journal.tsx` on `isAndroid && editingId()`.
Catalog + MFU are shared in `@outl/shared/toolbar` (port of `swift/OutlKit/Toolbar/*`); the action ids are the `window.__outlToolbar(action)` wire contract, so the Swift and TS catalogs stay byte-identical until the native bar retires.
Convention (shared `dispatchToolbarAction`, the two invariants): [`docs/clients.md` → Keyboard accessory bar](clients.md#keyboard-accessory-bar-mobile).

## Code execution (`run_code_block`)

Long-press a `` ```lang …``` `` block → "Run `<lang>`" fires `runCodeBlock`.
Mobile's `src-tauri/src/exec.rs` is a **thin adapter** over `outl_actions::exec::run_code_block` (shared with desktop), wrapping the outcome with a refreshed `PageView`.
The action only shows when `detectFence` matches; the backend re-validates in `run_block_at_index`, so a false-positive is a toast, not damage.
Runtimes on iOS: **Lisp, JS, Python, Lua** — `lang-rust` is off in `Cargo.toml`.
Flow + runtime-catalog rationale: [`docs/clients.md` → Running code blocks](clients.md#running-code-blocks).

## Insert template (structural templates)

The block long-press menu's "Insert template" action opens `TemplateSheet` (bottom sheet listing `listTemplates()`); picking one calls `instantiateTemplateAt(name, blockId)` and applies the returned `PageView`.
Wire commands are the shared `list_templates_cmd` / `instantiate_template_at` bodies — no mobile logic; contract in [`docs/clients.md` → Structural templates](clients.md#structural-templates).
