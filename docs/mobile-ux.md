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

## What's what — the first-run walkthrough

One sheet naming the bottom bar, the `+`, the week strip, the month label and the block long-press ([issue #574](https://github.com/outlmd/outl/issues/574)).
It opens once per device on a first launch and is re-openable from **Settings → What's what**.

It is not the 30-second tour (`firstRunTips`, `@outl/shared/polish`), and the split is the point.
That one teaches the **outline** (every line is a block, `[[` links a page) and only ever appears on an empty page, so a user whose journal already has content never sees it — and it names no chrome in any case.
Hence a key of its own (`CHROME_TOUR_KEY`); sharing `TIPS_KEY` would hide this from exactly the person who has been tapping unlabelled buttons for a week.

Two rules it keeps:

- **Nothing in it has to be completed before writing.** No steps, no next, no progress. The backdrop, the drag handle and the button all close it.
- **Any way out counts as read.** Someone who swiped it away has read as much of it as they intend to, and showing it again next launch would be the app not listening.

The copy is mobile-only (`lib/chrome-tour.ts`) because every row names a control that exists on this client; what is shared is the "show it once, then remember" mechanism, which never becomes an Op.

## Opening a ref that may not exist yet

`[[avelino/outl]]`, `[[2026-06-04]]`, `#code-review`, picker entries — every "tap a ref → see a page" path on the frontend goes through **one** Tauri command, `open_ref(target)`, which wraps `outl_actions::page::open_or_create_by_ref`.
The single decision tree (date → journal, else literal/slugified/title match → existing page, else create as page) lives in the shared crate so a frontend regex cannot drift from a backend parser the way it did before `open_ref` existed.

What used to be wrong: the frontend split the journal-vs-page decision with `/^\d{4}-\d{2}-\d{2}$/` and routed to one of two strict-validating commands (`open_journal_for` / `open_page_by_slug`).
`[[2026-13-01]]` matched the regex, hit `open_journal_for`, and surfaced an `invalid date slug` toast — even though falling through to "create a regular page" was clearly the right behaviour.

`open_page_by_slug` is kept for the picker (the picker already hands the command a clean slug from a known page).
`open_journal_for` stays for date-navigation commands (`previousDay` / `nextDay`) whose input is derived from controlled state, not from a user tap.
Every **ref-click** code path on the frontend (`handleRefClick`, `handleTagClick`) must call `openRef` so the decision tree is single-sourced.

`resolve_ref` survives for autocomplete previews ("this ref will land on `<page>`") but is **not** the navigation entry point — for that, always call `openRef`.

## What a `((blk-…))` ref shows

A ref renders the source block's text, and an embed renders it with its subtree.
`MarkdownInline` decides that purely from its `embeds` prop, so the question is only ever *who filled the map*.
`Journal.embeds.ts` owns the answer here: it collects the handles the open outline carries and asks `resolve_embeds` for them.

It is not asked on every commit.
Resolving rebuilds the workspace index, and `applyView` runs after every mutation, so a resolve happens only when the page or the handle set changed — typing a new ref grows the set, navigating changes the page, and both still resolve.
An older answer that lands after a newer one is dropped, since two resolves genuinely race (no workspace lock, a blocking pool) and the skip key already names the newer page, so letting the stale map win would leave the new refs raw until the next edit.

Two places still show the raw handle, on the desktop too:

- a ref inside a **backlink** row, because the handle collection walks the outline and a backlink's source block is not in it;
- a ref whose source block you are editing **on the page you are looking at**, because that changes neither the slug nor the handle set.
  It settles when you navigate away and back.

A failed resolve is silent on purpose.
The fallback is the raw handle, and a toast here would fire on every commit while a peer is mid-sync.

## Tag autocomplete (`#`)

Typing `#` plus a word opens the same chip strip `[[` opens, listing pages, because a tag is a page.
Tapping a chip inserts the page's **slug**, not its title: `#` has no closing delimiter, so the first space ends the tag and a title like "Deep Work" would tag `#Deep` and leave "Work" in the block as prose.
The chip still shows the human title.

`((` is deliberately absent from this trigger.
The native bridge speaks `PageMeta`, and a block hit is a handle plus a snippet, which is a different shape ([#15](https://github.com/outlmd/outl/issues/15)).

## Search sheet — what one box lists, and when it asks the backend

Pages, Tasks, Blocks and Actions in one pass, ordered by `@outl/shared/palette`; a leading `>` narrows to Actions.
Each section opens at three rows with the rest behind its "N more", **except Pages with an empty query**, which opens at 25.
With no query that list is the whole graph in slug order rather than a ranking, so three rows showed three pages from whatever sorts first.
ISO journal days sort last, so a journal-heavy workspace opened on 2019 and nothing recent was reachable without typing.
25 is the cap the ranked path already uses.

Both backends are debounced at 150 ms off one timer: `search_blocks` rebuilds the workspace index from disk per call, and `search_pages` rebuilds every `PageMeta` under the workspace lock before it scores anything.
An empty query loads immediately, since there are no keystrokes behind it to coalesce.
Blocks additionally wait for the query to reach `BLOCK_QUERY_MIN`.

## Search sheet — starting a page that is not a daily note

The `+` beside the search box opens a name sheet and `openRef`s whatever is typed.

It exists because there was no "New page" control at all ([issue #573](https://github.com/outlmd/outl/issues/573)).
Named pages have always worked.
The only way to *start* one was to type `[[` into a block and tap the link, which a phone-only user has no reason to know.
So writing in the journal every day led to the reasonable conclusion that every note has to live on a day.

Three things it does not do, each deliberate:

- **It does not decide what the name means.**
  `openRef` runs the canonical tree — a date → that journal, an existing literal / slugified / title match → that page, otherwise a new page titled with what was typed.
  So **a name that already exists opens that page** rather than making a second one.
  Branching on the shape in TypeScript first is the regex-vs-parser drift that command exists to remove.
- **It does not leave a `[[link]]` on today.** Opening the page is the whole v1; a page reachable only by search is a real concern, and it is the one `[[` already answers for anyone who wants the backlink.
- **It is hidden while the list is picking a merge target.** There is nothing to merge a page into that does not exist yet.

The sheet is `PageNameSheet` with `submitUnchanged`, the one prop this flow added.
A rename refuses a name equal to what it opened with, because that write does nothing; a create prefilled from the search box is exactly the case where submitting it unchanged is the point.

## Page switcher — long-press to delete

`PageSwitcher.tsx` renders each page as a row button; spreading `longPressHandlers(p)` arms a 500 ms sustained-touch detector (canceled if the finger moves more than 10 px).
On fire, `handleDelete(p)` runs `window.confirm(...)` → `deletePage(slug)` → navigates to the returned today's journal → refetches the list.
Journals are excluded — only regular pages can be deleted from the switcher.
The backend command is the shared `outl_tauri_shared::commands::page::delete_page` body — no mobile-specific logic.
`Action::DeletePage` carries a `g d` chord in the shared catalog (Normal mode), but mobile has no keyboard surface — long-press in the page switcher remains the only trigger on touch devices.

`BacklinksSection.tsx`'s `order`/`onToggleOrder` flips order via `setBacklinksOrder` (returns `PageBacklinks`); backlinks are lazy in `Journal.tsx` via `createResource(slug, pageBacklinks)` since `PageView.backlinks` is empty.

**Every section below the outline stays blank until the scan answers for the page on screen.**
The backlinks card used to mount immediately on a regular page and say "Linked from 0" / "No backlinks yet" while the scan ran, then contradict itself — `0` is a wrong answer, not a pending one.
A navigation showed the page the user just left answering for the one they opened, because Solid's `createResource` keeps its last value through a source change; `Journal.tsx` blanks it while `loading` for that reason.
Once the scan replies, a regular page keeps the empty box (a newcomer discovers that pages can cite each other) and a journal stays hidden.
`lateSectionVisible` (`@outl/shared/outline`) owns the rule; `PageSections.test.tsx` is the net.

## Backlink rows — a place you can work

A backlink row shows a block that lives on another page, and tapping it **edits that block in place** rather than navigating.
The editor is this client's own `EditableTextarea`, through `<BacklinkRowEditor />`, so the iOS text-input rules (`beforeinput`, `parkCaret`, no write while an IME composes) come with it.
**The commit is on blur**, never per keystroke: it writes to another page and then re-asks an `O(blocks-in-workspace)` scan.
The row keeps showing the editor until the fresh rows are in, so you never watch your edit revert and reappear.

The **checkbox** toggles the task on the source page — it used to draw the state and do nothing, and the way out it offered was to leave for that page.
The **card header** is what still opens the source page.

A **long press** opens the same `<BlockContextMenu>` the outline raises, with task state, indent, outdent, move up / down, create below / above, and delete.
Seven of those rows take their label and glyph from one shared table, so an action cannot look like two things depending on which sheet you are in.
Create below / above is the one pair that does **not** raise the keyboard: the new block cites nothing, so no row here will ever render it and there would be no textarea to focus — you are told which page it landed on instead.
Delete asks when the block has children, in the same prompt the outline uses; a citing block's children are drawn under it, so the count is real.
Each child is its own tap and long-press target (44 pt minimum) with its own sheet.

**The card takes one write at a time.**
The acting row dims and goes `aria-busy` while its write is in flight, and a gesture on another row of the card is dropped in that window.
None of these is idempotent from your seat — a second toggle flips the task back — and a timeout would be worse than no guard, since firing one while the write still lands means the task flips twice on the retry.

A row whose citing block lives outside any page is **inert**: no editor, no sheet, no jump, because there is no page to address.

Contract, including why a cross-page edit refuses rather than keeping your text somewhere else: [`client-backlinks.md` → Editing a backlink where it is cited](client-backlinks.md#editing-a-backlink-where-it-is-cited).

## Page title - tap to rename

**Tapping the title of a page opens a name sheet**, prefilled with the current name and selected so one keystroke replaces it.
A small pencil glyph sits beside the title so the gesture is findable; it never appears on a journal or on the launch placeholder, so it cannot shift the title after the fact.
Submitting the name unchanged, or empty, does nothing: the Rename button stays disabled and no command is sent.

**A journal title still means "back to today".**
Its slug is its date, so the rename is refused by the backend, and an affordance that exists only to be refused is the defect rather than the guard.
Press-and-hold on the title still opens page properties on both kinds, unchanged, and a completed hold no longer also opens the rename sheet.

**The field is a sheet, not an input in the header, and that is load-bearing.**
`document-scroll-lock.ts` stands down for a focused `INPUT`, because there the keyboard owns the scroll.
That is right for the outline's textareas and wrong for a field in the chrome: WKWebView scrolls the document to keep the caret above the keyboard, and what it scrolls away is the header the field is in.
That is the journal header riding up.
`PageNameSheet` is `position: fixed` against `bottom-0` with `keyboardInset()` padding, outside the document flow, so it cannot reproduce it.
The tap handler also calls `armKeyboard()` **before** the sheet renders, the pattern [the keyboard handoff](#making-a-block-and-typing-in-it-the-keyboard-handoff) records.
The sheet focuses inside `requestAnimationFrame`, one frame past the gesture window, so from a cold keyboard WKWebView would otherwise raise nothing.

When the name is taken, the app's own alert asks whether to merge this page into the one that holds it, in the same words the page switcher's rename uses.
Accepting merges the page you were looking at **into** the existing one and opens that page; declining writes nothing and goes nowhere.

## Page switcher - rename and merge

The long-press sheet that offers delete also offers **Rename page** and **Merge into...**, for regular pages only: journals are excluded, because a journal's slug is its date.
The commands are the shared `rename_page` / `merge_pages` bodies, so every link to the page is rewritten in the same batch ([RFC 0557](rfcs/0557-page-merge-and-rename.md)).

- **Rename** opens a text prompt prefilled with the current title.
  When the new name already belongs to another page, nothing is written and a confirm asks "A page named X already exists. Merge this page into X?".
  Accepting runs the merge, cancelling leaves both pages as they were.
- **Merge into...** opens a page picker without the source page, and always asks for a confirmation before writing.
- **Afterwards** the app opens the resulting page (renamed, or the merge target) and shows a summary with blocks moved, links updated and any property conflicts (the target's value is kept, the dropped one is named).

There is no undo for either action, and a merge does not come apart: see [RFC 0557 → The opposite direction](rfcs/0557-page-merge-and-rename.md#the-opposite-direction).
Neither has a keyboard chord.
A rename has a second door, the [page title](#page-title---tap-to-rename); a merge picked from the target side is reached only here, because the header has no list to turn into a picker.
`Capability::PageRename` and `Capability::PageMerge` record the verdicts in [client-parity.md](client-parity.md).

## Opening an external `[label](url)` link

Tapping an external link opens it in the system browser via **`tauri-plugin-opener`** (registered in `lib.rs`, capability `opener:allow-open-url` for `http(s)`/`mailto`).
`Journal.tsx`'s `handleLinkClick` calls the shared `openExternalUrl` — same as desktop, so the allow-list (`http(s)`/`mailto`; `file:`/`javascript:` rejected) lives in one place.
`<MarkdownInline />` gets `onLinkClick` threaded from `Journal.tsx` → `BlockRow` → the renderer.
An `assets/…` link routes instead (via `isAssetLink`) to `openAsset` — `open_asset` opens the file in the OS viewer.
The block long-press **Attach file** action picks a file (`@tauri-apps/plugin-dialog`) → `attachAsset` (shared `commands::asset`).
On iPad, dragging a file onto a block imports it the same way via the shared `installFileDrop` + `importAssetFile` (`@outl/shared/drag-drop`), best-effort — iPhone rarely delivers a webview drop, so long-press stays the only import path there.
`[[ref]]`/`#tag` taps in the outline still route through `openRef`.
A **backlink** row's own tokens do not: the row's one gesture is editing the block it shows, so `<MarkdownInline>` there gets no `onRefClick` / `onLinkClick` and a ref inside it renders without navigating.

## Properties (`key:: value`) on a block

A block's properties render as **one attribute line** under its text: key in a dim grey, value a step darker, `·` between fields, truncated, never more than a line.
Past two fields the rest reads `+N more`.

Tapping anywhere on the line opens the properties sheet for that block; the page's own properties sit in the same kind of line above the outline and open the sheet on its Page side.
There is no inline editing in the outline — the sheet is a two-column field list, and a row that looks like a form row is the one that opens a form.

**Only a field that acts on you gets colour.**
Today that is `remind::`, which shows a bell and the accent and drops its key, since the icon already says what it is.
`auto-run::` runs code and still renders quiet: the criterion is whether it will interrupt you, not whether it matters.

**An importer's stamps do not render here at all.**
`created`, `edited` and `roam-uid` arrive from `outl import` and are on essentially every block of a migrated graph.
They are still in the `.md` and still listed in the sheet — dimmer, and below what you wrote — because deleting one means seeing it first.
A `created:: by Roberta` you typed yourself is not a stamp and renders normally; only an ISO-8601 value counts.

## Remind me — authoring a `remind::` rule

A long-press offers **Remind me…**, which opens `<RemindSheet />` rather than the `window.prompt` it used to ([issue #572](https://github.com/outlmd/outl/issues/572)).
Time, repeat, an "until" that only appears for a rule that repeats, and an **Edit as text** field holding the rule verbatim.
Why both halves exist, and why the text field is what keeps the picker honest: [`docs/reminders.md` → Mobile](reminders.md#mobile-outl-mobile).

What is decided in this client, and nowhere else:

- **The rule text is never written here.** `parseRemindRule` on open, `formatRemindRule` on save, both over `outl_md::remind`. Two round-trips per session, not per keystroke.
- **A rule that does not parse opens in the text field**, carrying what the block actually says. Dropping the user into the controls would show them a 09:00 that has nothing to do with their rule.
- **An interval the list does not offer keeps its own label** (`Every 45min`), because a list with nothing selected reads as "Once" and silently drops the repeat on the next save.
- **Switching the "until" clause seeds it** — a time one hour past the anchor (a stop at or before it is a clause the parser drops), or today's date — so Save is never a button that fails on a half-filled field.
- **Remove is a button**, not an empty value the user has to discover. It is the only way to stop a block nagging without deleting it.

The Reminders tab's empty state names this gesture, so opening the tab with nothing scheduled is not a dead end.

## Blockquote chrome

A `"> "`-prefixed block gets a left border + ~5% tint, right-rounded, body full-colour (refs / bold / tags keep their palette).
The outline bullet and `<CollapseTriangle />` stay outside the quote chrome; a non-quoted block degrades to a plain flex container (byte-identical).
Detection is `splitQuote` + `stripQuoteFromTokens` (`@outl/shared/markdown`, mirror of `outl_actions::quote::split_quote`) so the `> ` isn't rendered twice; it composes with the task checkbox.
The checkbox has three filled states: DONE is a solid accent circle with a tick, **DOING is an accent ring with a small filled centre**, TODO is a neutral empty ring.
One tap walks one stop of `TODO → DOING → DONE → none`.
**Nothing in this client toggles it**: a quote is authored by typing `> ` at the front of the block, and the long-press sheet has no quote row.
The route is wired and this client is not on it: `toggleQuote` (`@outl/shared/api/commands`) → the registered `toggle_quote` command → `outl_actions::block::toggle_quote`, which the desktop reaches through `/quote`.
So adding a sheet row is a caller rather than a feature, and it needs no TS string surgery when it lands.
Recorded as `Capability::BlockquoteToggle` → `❌` on mobile in [`client-parity.md`](client-parity.md); the nudge names the marker to type instead.
Convention (three-surface parity): [`docs/client-editing.md` → Blockquote convention](client-editing.md#blockquote-convention).

## Zoom / focus on a block

Tap a block's plain bullet dot to zoom in — it becomes the outline root (Roam/Workflowy focus); `← Back` + breadcrumb zoom out.
Local view state (`focusBlockId` in `Journal.tsx`), never a Tauri round-trip; the shared `focusSubtree` (`@outl/shared/outline`) does the subtree + breadcrumb.
Mobile owns only the touch chrome: the bullet tap moves mark-as-TODO to the long-press menu; checkbox + `<CollapseTriangle>` untouched.

**Nothing that belongs to the page is on screen while zoomed**: the backlinks card, the nested-pages list, the namespace mentions and the page's own `key:: value` row all go, because the header is a *block* there.
**Opening a page leaves the zoom, including the page already open** — Today, the week strip, the calendar, a tap on a `[[ref]]`, the page switcher, a deep link and wake-to-today.
A commit, a background poll, a peer reload and a capture into today keep it: none of those is the user asking to go somewhere.

Convention: [`docs/client-editing.md` → Zoom / focus on a block](client-editing.md#zoom--focus-on-a-block-roamworkflowy).

## The bullet opens the menu too

A **hold on the bullet** (or on a task's checkbox) raises the same `<BlockContextMenu>` a hold on the row's text does.
It did not, for a reason nothing on screen explained: `BlockBody.pressedInteractive` refuses to arm the hold inside any `a` / `button` / `[role=button]` / `code` / `textarea`, and the bullet is a `<button>`.
So the one place a finger reaches for "act on this block" was the one place the gesture died.

The exemption is that control alone — `[data-block-bullet]`, set in `BlockMarkers.tsx` on both the dot and the checkbox — rather than a looser predicate.
A ref, a tag, a link, inline `code`, the fold triangle and the textarea each still handle their own touch, and a hold on any of them still opens nothing.

**A tap is unchanged**: the dot zooms (or marks TODO where zoom is not wired), the checkbox cycles task state.
What the hold needed was a second reader of `consumedClick()`.
Those two buttons `stopPropagation` unconditionally, so the row's `onClick` never sees their click and cannot swallow the one a completed hold produces.
Without it, holding the bullet opened the menu *and* zoomed on the way out.
`BlockBody`'s `bulletTap` is that reader, which keeps it at exactly one per gesture.

## Copy block ref / Copy block embed

Two adjacent rows in the long-press menu, and the difference is what a reader sees after pasting.
**Copy block ref** writes `((blk-XXXXXX))` — a link to the block.
**Copy block embed** writes `!((blk-XXXXXX))` — the block expanded in place.
Both read the handle the workspace index already assigned, and both refuse with "no ref handle yet — save and retry" on a block with no sidecar entry.

The `!` is added by `outl_md::inline::block_embed_token`, never by prefixing the ref's result in a client: the GUI pair is the TUI's `/refer` and `/refer-embed`, and a second owner of the syntax is the one that drifts.

## Paste from external apps

The textarea in `BlockRow.tsx` intercepts paste (with formatting only — mobile has no `Cmd+Shift+V`).
Rich `text/html` converts via `htmlToOutlMarkdown` (`@outl/shared/paste`); plain text routes to `paste_markdown_at` when `looksLikeOutline` **or** `hasMultipleParagraphs`, splitting multi-paragraph into one block each (else native splice).

A paste that merges into the block being edited comes back carrying that block's new text (`PasteReply`), and `pasteMarkdownAt` in `Journal.block-ops.ts` writes it into the live textarea before the user keeps typing.
Skipping that was [issue #488](https://github.com/outlmd/outl/issues/488): `draft` is seeded once, so the editor kept the pre-paste string and `commitEdit` wrote it back over the paste on the next blur, losing a pasted link from the block and from the `.md` with nothing said.

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

Full behaviour and the per-client table: [`client-outside-the-app.md` → Capturing from outside the app](client-outside-the-app.md#capturing-from-outside-the-app).
Android's components, the JNI boundary and why they need no "a capture arrived" notification: [`android-platform.md`](android-platform.md#capture-from-outside-the-app).

## Arrow keys walk between blocks (hardware keyboard)

With a keyboard attached — an iPad, or an Android phone with one — the arrows do not stop at the edge of the block being edited: the outline reads as one continuous document, the same way it does on the desktop and in the TUI.

| Keystroke | Where the caret goes |
|---|---|
| `←` with the caret at offset 0 | end of the block above |
| `→` with the caret at the end | start of the block below |
| `↑` on the block's first line | block above, same column |
| `↓` on the block's last line | block below, same column |

The draft is saved on the way out, and the step waits for that save: a failed write leaves the user on their text with the error, rather than walking away from it.
At the top or bottom of the outline the key is handed back to the webview, so it still moves the caret inside the block instead of doing nothing.
A modified arrow never walks, and an arrow with text selected collapses the selection.

The soft keyboard has no arrow keys, so on a bare phone none of this is reachable — which is why it is not in the accessory bar either (that catalog is the `window.__outlToolbar` wire contract shared with Swift, and adding a row to it is a change to both).
Decision shared with the desktop in `@outl/shared/caret-nav`, including the one limit: a *line* is a logical `\n` line, not a soft-wrapped visual one.

## Keyboard accessory bar (Android web bar / iOS native bar)

The keyboard toolbar + suggester strip have two renderings.
iOS is native (`OutlToolbarView` swizzled onto `WKContentView`), untouched.
Android is web: `KeyboardAccessory.tsx` → `<SuggesterStrip />` + `<KeyboardToolbar />`, gated in `Journal.tsx` on `isAndroid && editingId()`.
Catalog + MFU are shared in `@outl/shared/toolbar` (port of `swift/OutlKit/Toolbar/*`); the action ids are the `window.__outlToolbar(action)` wire contract, so the Swift and TS catalogs stay byte-identical until the native bar retires.
Convention (shared `dispatchToolbarAction`, the two invariants): [`docs/client-editing.md` → Keyboard accessory bar](client-editing.md#keyboard-accessory-bar-mobile).

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
Runtimes on iOS: **JS, Python, Lua**. `lang-rust` is off in `Cargo.toml`, and `lang-lisp` is off everywhere by default (see [`docs/client-editing.md`](client-editing.md#running-code-blocks)).
Flow + runtime-catalog rationale: [`docs/client-editing.md` → Running code blocks](client-editing.md#running-code-blocks).

## Insert template (structural templates)

The block long-press menu's "Insert template" action opens `TemplateSheet` (bottom sheet listing `listTemplates()`); picking one calls `instantiateTemplateAt(name, blockId)` and applies the returned `PageView`.
Wire commands are the shared `list_templates_cmd` / `instantiate_template_at` bodies — no mobile logic; contract in [`docs/client-editing.md` → Structural templates](client-editing.md#structural-templates).
