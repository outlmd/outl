# Mobile UX behaviour

How `outl-mobile` behaves, feature by feature.

Moved out of `crates/outl-mobile/CLAUDE.md`, which is loaded into the model's context on every session in that crate and had grown past the point where that is useful.
These sections are reference: you read one when you touch that feature, not every time you open the crate.
The rules a contributor must not break stayed in `CLAUDE.md`.

Platform plumbing has its own homes: [`docs/ios-platform.md`](ios-platform.md), [`docs/android-platform.md`](android-platform.md).
Anything visual — colours, tokens, spacing, components — is specified in [`DESIGN.md`](../DESIGN.md).
The behaviour rules these sections are instances of — the interaction patterns, the voice a nudge has to pass, which state converges — are in [`UX.md`](../UX.md).

## Bottom bar

Four places and a capture button: Journal, Search, Tasks, Reminders.

- **Journal** jumps to today.
- **Search** opens the one search box over pages, tasks and blocks (issue #364). There is no separate Pages tab.
- **Tasks** opens the task list.
- **Reminders** opens the reminders sheet.
- **+** in the middle writes to today's journal, wherever you are.

Reminders is a tab because it had only a header glyph and a palette entry, and a glyph is not a name.
Tapping a tab closes the other sheets.
The sheets cover the bar, so the lit tab is only visible for Journal.
The bar hides while a block is edited or a range is selected, because the keyboard toolbar owns that space.

## Opening a ref that may not exist yet

`[[avelino/outl]]`, `[[2026-06-04]]`, `#code-review`, picker entries — every "tap a ref → see a page" path on the frontend goes through **one** Tauri command, `open_ref(target)`, which wraps `outl_actions::page::open_or_create_by_ref`.
The single decision tree (date → journal, else literal/slugified/title match → existing page, else create as page) lives in the shared crate so a frontend regex cannot drift from a backend parser the way it did before `open_ref` existed.

What used to be wrong: the frontend split the journal-vs-page decision with `/^\d{4}-\d{2}-\d{2}$/` and routed to one of two strict-validating commands (`open_journal_for` / `open_page_by_slug`).
`[[2026-13-01]]` matched the regex, hit `open_journal_for`, and surfaced an `invalid date slug` toast — even though falling through to "create a regular page" was clearly the right behaviour.

`open_page_by_slug` is kept for the picker (the picker already hands the command a clean slug from a known page).
`open_journal_for` stays for date-navigation commands (`previousDay` / `nextDay`) whose input is derived from controlled state, not from a user tap.
Every **ref-click** code path on the frontend (`handleRefClick`, `handleTagClick`) must call `openRef` so the decision tree is single-sourced.

`resolve_ref` survives for autocomplete previews ("this ref will land on `<page>`") but is **not** the navigation entry point — for that, always call `openRef`.

## Search sheet — what one box lists, and when it asks the backend

Pages, Tasks, Blocks and Actions in one pass, ordered by `@outl/shared/palette`; a leading `>` narrows to Actions.
Each section opens at three rows with the rest behind its "N more", **except Pages with an empty query**, which opens at 25.
With no query that list is the whole graph in slug order rather than a ranking, so three rows showed three pages from whatever sorts first.
ISO journal days sort last, so a journal-heavy workspace opened on 2019 and nothing recent was reachable without typing.
25 is the cap the ranked path already uses.

Both backends are debounced at 150 ms off one timer: `search_blocks` rebuilds the workspace index from disk per call, and `search_pages` rebuilds every `PageMeta` under the workspace lock before it scores anything.
An empty query loads immediately, since there are no keystrokes behind it to coalesce.
Blocks additionally wait for the query to reach `BLOCK_QUERY_MIN`.

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

## Making a block and typing in it (the keyboard handoff)

Tapping "+" has to leave the user typing, not looking at an empty bullet.

WKWebView raises the software keyboard for a programmatic `.focus()` only while the user-gesture window opened by the tap is still open, and creating a block is two awaits deep: commit whatever draft is open, then `create_block` over IPC.
So `<EditableTextarea>`'s `onMount` focus always landed too late — the block appeared, the textarea really was focused (the accessibility tree reported `AXTextField [focused]`), and no keyboard came up.
Neither did the native accessory bar, which is the same signal one layer up.

`lib/keyboard-keeper.ts` is the fix and the single owner of it: a 1×1, `opacity: 0` textarea appended to `document.body` once at first paint.
A tap focuses it **synchronously**, which raises the keyboard; the awaits then run with the keyboard already up; and when the new block's textarea mounts and focuses, iOS sees a move between two text inputs and leaves the keyboard alone.

Three things about it are load-bearing, and each is a way a later "let's hide that properly" breaks it:

- It must stay laid out. iOS raises no keyboard for a `display: none`, `visibility: hidden`, `disabled` or `readonly` field, so `opacity: 0` is the most hiding the trick survives.
- It must live outside the outline, because applying the new `PageView` unmounts the empty state the tap came from and a keeper inside that subtree would be destroyed mid-handoff.
- `font-size: 16px`, or iOS zooms the page when it takes focus.

**Anything typed during the round-trip is carried, not dropped.**
The keeper holds those characters and `takeArmedText()` seeds the new block's draft with them, which reaches the op log as an ordinary edit against the id `create_block` returned.
There is no block id before the backend replies, and the keystrokes are never held against an invented one.
If the create fails, the keyboard is given back rather than left up over an invisible field.

Which paths arm it, and which do not need to:

| Path | Keyboard before the tap | Arms |
|---|---|---|
| "+" on today's journal, "Add a block", "Tap to start writing", "Start writing" in the first-run tips | down | yes, in `appendBlock` |
| Long-press → "New block above" | down | yes — the menu arms it synchronously via `armsKeyboard` on the action, because `BlockContextMenu` defers `onSelect` past the gesture |
| "+" anywhere else → `<CaptureSheet />` | down | yes, before the sheet opens (the sheet's own focus is one frame later, already outside the gesture) |
| Return inside a block, the accessory bar's "New line" (`handleCreateAfter`) | up | no — `armKeyboard()` would be a no-op anyway, and stealing focus from the open textarea would commit its draft |
| Tapping a block to edit it | down | no — that path mounts and focuses in the same task as the tap, which is why it always worked |

## Capturing without opening the app

Three OS-level surfaces per platform, and none of them needs the app running.
Each writes one file into an inbox, and the app lands it on launch and on every return to the foreground.

| | iOS | Android |
|---|---|---|
| Home screen | widget → opens the capture sheet | widget → opens the capture sheet |
| Share sheet | share extension → writes the line | share target → writes the line |
| One-tap | "Add to today" App Intent (Shortcuts, Back Tap, Action button) | quick settings tile → opens the capture sheet |

The widgets and the tile write **nothing**: they fire `outl://capture`, which opens the in-app `<CaptureSheet />`.
Only the share surfaces write a file, which is why they are the ones that work with the app closed.

What lands is always **one block**, at the end of the journal for the day the capture was *made*.
Sharing a multi-line selection from a browser gives you those lines joined by spaces, not a block with newlines in it.

**Nothing is lost if you never open the app** — the captures wait in the inbox on that one device, unsynced, until a drain picks them up.
A file that cannot be read stays there and is counted in what the app reports after a drain.

The in-app quick capture (`+` → `<CaptureSheet />`) is a different thing and is unchanged: it writes through the op log directly, because the app is already open.

Full behaviour and the per-client table: [`clients.md` → Capturing from outside the app](clients.md#capturing-from-outside-the-app).
Android's components, the JNI boundary and why they need no "a capture arrived" notification: [`android-platform.md`](android-platform.md#capture-from-outside-the-app).

## Keyboard accessory bar (Android web bar / iOS native bar)

The keyboard toolbar + suggester strip have two renderings.
iOS is native (`OutlToolbarView` swizzled onto `WKContentView`), untouched.
Android is web: `KeyboardAccessory.tsx` → `<SuggesterStrip />` + `<KeyboardToolbar />`, gated in `Journal.tsx` on `isAndroid && editingId()`.
Catalog + MFU are shared in `@outl/shared/toolbar` (port of `swift/OutlKit/Toolbar/*`); the action ids are the `window.__outlToolbar(action)` wire contract, so the Swift and TS catalogs stay byte-identical until the native bar retires.
Convention (shared `dispatchToolbarAction`, the two invariants): [`docs/clients.md` → Keyboard accessory bar](clients.md#keyboard-accessory-bar-mobile).

### The row only changes shape between editing sessions

MFU reorders the middle range, and both bars used to reorder it **on every tap** (`rebuildButtons()` in Swift, a re-read signal in `KeyboardToolbar.tsx`).
That moved the button the user had just hit out from under their finger before they could hit it again, so indent-indent-indent landed on three different buttons ([#269](https://github.com/outlmd/outl/issues/269)).
Now the order is resolved once per editing session and held: the web bar reads it at mount (its parent gates it on `editingId()`, so a mount *is* a session), and the native bar reads it when the keyboard **appears** — `OutlSwizzle` keeps one `OutlToolbarView` for the whole app lifetime, so a keyboard notification is the only thing that marks a session boundary there.

`keyboardWillShowNotification` is not one-per-session, which is the trap: iOS re-posts it while the keyboard is already up (input-mode switch, emoji, the QuickType bar appearing — which this app keeps on).
Acting on every post puts the mid-session reshuffle straight back, so `OutlToolbarView` tracks `keyboardVisible` and rebuilds only on the rising edge.
`OutlSuggestOverlay` already kept the same flag for the same reason.
Taps are still counted the moment they happen; MFU just gets to act on the count at the next session rather than mid-gesture.

### Locking the order (`SettingsSheet`)

The header's gear opens `components/SettingsSheet.tsx`, mobile's first **preferences** surface (every other sheet shows workspace content).
It opens on **Devices and sync**, which is how pairing is reached by name: the sync dot's press-and-hold does the same thing faster, and a gesture alone left the feature looking absent to anyone who went looking for it.
It carries two toolbar controls: "Lock button order", which freezes the row the user has today, and "Reset button order", which forgets the MFU counts.
Reset re-freezes on the cold-start order when the bar is locked — a locked bar renders its snapshot, so wiping the counts alone would change nothing on screen, and the user asked for the default back, not to be unlocked.

Both are **per-device UI state in `localStorage`**, never the op log: two devices disagreeing about their own toolbar layout is not a conflict to reconcile (root `CLAUDE.md` invariant 7).

**Neither the lock nor the tap counts live in `UserDefaults`, and that is what makes these two buttons work on iOS.**
The bar there is native, but the sheet that locks and resets it is web.
While the counts sat in `UserDefaults`, the sheet read an always-empty store: "Lock button order" froze the *cold-start* row rather than the user's own — reintroducing the very reshuffle it exists to stop — and "Reset button order" did nothing at all.
So `localStorage` is the single home for both, `Journal.tsx`'s `dispatchToolbarAction` is the single counter (every tap from either bar passes through it), and `OutlToolbar.swift` reads the pair back with one `evaluateJavaScript` when the keyboard appears.
A failed read keeps the previous values rather than silently unlocking a bar the user locked.

The two storage keys are pinned on both sides — `ToolbarStoreTests` in Swift, `keys.test.ts` in TypeScript — because nothing fails at build time when one language renames one: the bar just stops seeing the user's taps and lock, silently.

## Code execution (`run_code_block`)

Long-press a `` ```lang …``` `` block → "Run `<lang>`" fires `runCodeBlock`.
Mobile's `src-tauri/src/exec.rs` is a **thin adapter** over `outl_actions::exec::run_code_block` (shared with desktop), wrapping the outcome with a refreshed `PageView`.
The action only shows when `detectFence` matches; the backend re-validates in `run_block_at_index`, so a false-positive is a toast, not damage.
Runtimes on iOS: **JS, Python, Lua**. `lang-rust` is off in `Cargo.toml`, and `lang-lisp` is off everywhere by default (see [`docs/clients.md`](clients.md#running-code-blocks)).
Flow + runtime-catalog rationale: [`docs/clients.md` → Running code blocks](clients.md#running-code-blocks).

## Insert template (structural templates)

The block long-press menu's "Insert template" action opens `TemplateSheet` (bottom sheet listing `listTemplates()`); picking one calls `instantiateTemplateAt(name, blockId)` and applies the returned `PageView`.
Wire commands are the shared `list_templates_cmd` / `instantiate_template_at` bodies — no mobile logic; contract in [`docs/clients.md` → Structural templates](clients.md#structural-templates).
