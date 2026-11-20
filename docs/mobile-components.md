# Which mobile component owns what

`Journal.tsx` and `BlockRow.tsx` were the two largest files in the repo, and most of each is now a sibling module.
This is the map: what came out, what each piece owns, and which shape of new code belongs where.

Moved out of [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md), which is loaded into the model's context on every session in that crate.
These sections are reference: you read one when you touch that component.
The rules that hold whatever you are editing stayed in `CLAUDE.md`.

What the user experiences, feature by feature, is [`docs/mobile-ux.md`](mobile-ux.md).

## `Journal.tsx` and what has been split out of it

`Journal.tsx` is the mobile app's one large component and the single biggest file in the repo.
It reached 3,212 lines because the frontend sat outside `file-size-guard.sh`, which only read `.rs` until 2026-09 — so nothing ever warned.

Ten pieces are now siblings, and new code of any of those shapes belongs there rather than back in the parent:

- **`JournalHeader.tsx`** — `JournalHeader`, `PageHeader`, `ChevronLeft`, `ChevronRight`.
  Pure render: props in, markup out, no state and no commands.
- **`Journal.context-actions.ts`** — `buildContextActions` plus its only helper `locateSiblings`.
  A pure function from (block id, page view, handlers) to the typed row list `<BlockContextMenu>` renders.
  It has no Solid import at all, which is why it is `.ts` and not `.tsx`.
  `Journal.buildContextActions.test.ts` drives it directly.
- **`Journal.toolbar-dispatch.ts`** — `dispatchToolbarAction(action, handlers)`, the switch both keyboard bars fire through, plus the tap counting that feeds MFU and the settings sheet.
  Same shape as `Journal.context-actions.ts`: a pure function from (action, handlers) to effects, no Solid import, so it is `.ts` and `Journal.toolbar-dispatch.test.ts` drives it directly.
  It counts the tap **before** routing it, so a button pressed with nothing focused still registers — the user pressed it, which is what MFU measures.
- **`Journal.block-ops.ts`** — `createBlockOps(deps)`, every operation on **one** block: toggle TODO, delete (and its prompt), copy / paste / cut / copy-ref, indent, outdent, move, and the external-clipboard `pasteMarkdownAt`.
  Each structural op is the same four-step shape (page id → one command through `withError` → fold the `PageView` back in → haptic), which is why they belong together and why the shape is the module's contract. The clipboard ops are the exception: `copyBlock` and `copyBlockRef` resolve no page id and buzz for nothing, because neither mutates the workspace.
  Handed accessors, never values: `Journal` passes `pageId` / `view` / `editingId` as functions, because a destructured prop (or dep) freezes at first render in Solid.
  **No handler reads `this`**, which is what lets `Journal` hand every one of them to `<BlockRow />` as a prop: they are always called detached from the object, and a `this.` would break there silently. Most are method shorthand on the returned literal, which is fine for exactly that reason; `performDelete` is a named function because `requestDelete` calls it internally. `Journal.block-ops.test.ts` pins the detached case rather than the spelling.

- **`Journal.selection-ops.ts`** — `createSelectionOps(deps)`, the touch-native range selection (RFC 0254 phase 3) and every op over N blocks.
  Extends `JournalBlockDeps` because a range op is N single-block commands plus a range.
  Split from `Journal.block-ops.ts` on the **question** ("what is selected, and what happens to N blocks" vs "what happens to one"), not the line count — the standard `backlinks_index.rs` → `backlinks_keys.rs` set.
  The walk order is the subtle part and is pinned by test, not comment: indent / move-up walk top-down, **move-down and delete walk bottom-up** (a move-down has to clear the block below the range first; a delete has to take children before parents or the parent trashes the subtree and the follow-up fails).
  *What* the range contains is not decided here at all — `visibleRangeSlice` in `@outl/shared/outline` owns it, so this client and the desktop cannot disagree about it. The first version of this module carried a comment claiming it "mirrors the desktop's `applyVisualBlockOp` exactly", which is a parity invariant nothing could fail.

- **`Journal.listeners.ts`** — the four listeners the journal installs, over four backend events and one webview event, plus `navigateDeepLink` which the cold-start path calls directly.
  The backend events are `deep-link://navigate`, `ref-projection-failed`, and `projection-write-failed` + `workspace-ready` together.
  The fifth subscription is `installFileDrop`, a webview drag-and-drop event rather than a backend one — it breaks in the webview and is debugged there, not in the Rust bridge.
  **`installJournalListeners` is the one entry point the component calls**, the shape `installJournalPolls` already has, so which listeners exist is one fact in one place.
  Four call sites in `Journal.tsx` was four chances to add a listener here and never reach it, which is how `ref-projection-failed` had a subscriber on the desktop and nothing at all here.
  They must still be **called from inside the component body**: `onCleanup` needs a Solid owner, and there is deliberately no `runWithOwner` wrapper hiding that.
  The ordering rule they exist to hold in one place — arm `onCleanup` **synchronously**, before the dynamic `import()` resolves, then dispose a late-arriving handle — has exactly one implementation, `armCleanup()`. Registering cleanup inside the `.then()` instead leaks the listener on a fast unmount, invisibly; `Journal.listeners.test.ts` holds the import open, unmounts, and releases it, so that window is covered rather than described.
  Its `listen` mock **keeps the callback**, which is what makes the listener bodies testable: the deep-link mid-edit guard, the `workspace-ready` branch and the on-page / off-page routing of a refusal are each pinned by a test that fails when the branch is inverted. A mock that drops the callback leaves all three deletable with a green suite.

- **`JournalDeleteDialogs.tsx`** — both delete confirmations (single block, and range) plus the copy that goes in them.
  `Journal` keeps the pending-delete signals and the `performDelete*` calls; this only renders and reports back.
  The **wording** is what made the pair worth extracting together — pluralising a count, warning about children, "can't be undone" — since two copies of that drift into saying different things about the same action.
  The markup is one `<ConfirmDialog />` each and would not have earned a file on its own.

- **`PageSections.tsx`** — everything below the outline, in order: the backlinks card (`<BacklinksSection />`) and the nested-pages list (`<NestedPages />` from `@outl/shared/namespace`, issue #275).
  **It draws nothing while the outline is zoomed into a block** (`zoomed`, handed down from `Journal`'s `focusView()`), and `Journal` hides the page's own `key:: value` row on the same condition: the header is a *block* there, so the page's backlinks under it are a wrong answer rather than a noisy one.
  Both facts live in `Journal.tsx`'s JSX, so `PageSections.test.tsx` pins them by reading the source, the way the `backlinks` prop's own contract already is.
  The twin of the desktop's `PageSections.tsx`, and the reason the two sections live in one component rather than one inside the other: they look alike and answer different questions — backlinks point *at* this page, nested pages live *under* its namespace.
  `Journal` keeps the navigation, passing a single `onOpenPage(slug, kind)` that both sections route through, so the journal-vs-page opener choice has one owner instead of one per section.
  **A backlink's ancestor breadcrumb is `<CrumbTrail />` (`@outl/shared/markdown`), never a `join` of `crumb.text`** — a crumb carries inline tokens, so the trail renders like any other block and stays inert.
  The desktop's `InlineBacklinks` wraps the same component, which is what stops one client drawing markdown source while the other renders it; `BacklinksSection.test.tsx` is this side of that net.
That opener is `openView` (`Journal.navigation.ts`), not `applyView` — see that module's entry above.
  **Nothing here draws until the scan answers for the page on screen** (`lateSectionVisible`, `@outl/shared/outline`).
  `Journal` is what makes that true: `createResource` keeps its last value through a source change, so the raw accessor left the page the user just left answering for the one they opened, while the card said "No backlinks yet" over a scan still running.

- **A backlink row is a place you can work, not just a destination.**
  Tapping it opens `<BacklinkRowEditor />` over this client's own `EditableTextarea`; the card header is what still opens the source page, and the checkbox toggles the task there.
  `BacklinkCheckbox` **drew task state and performed nothing** until this landed — its own doc said so, and the way out it named was to leave for the source page.
  Which `(page, block)` a gesture targets is `@outl/shared/backlink-edit`'s — an orphan row has no page to address, so it is **inert**: no editor, no sheet, no jump.
  Three rules with a failure mode each.
  **`editBacklinkBlock` / `backlinkRowOp` / `createBacklinkSibling`, never the `block.rs` twin**, which tolerates a stale id — as a backlink's routinely is.
  The twin appends to a page the user cannot see, or re-parents a peer-trashed block *inside* the trash and reports success.
  **Commit on blur**, because the commit projects and then rescans `O(blocks-in-workspace)` (invariant 15).
  And **a refusal keeps the editor up on its draft**, since nothing was written.

  **The section reports, the host decides.**
  `onEditCommitted({ sourcePageId, view })` is the whole upward surface: this card never learns the open page's id, because `backlinkCommitPlan` needs *both* ids and only `Journal.backlinks.ts` holds them.
  A section taking `openPageId` plus an `onApplySourceView` was one question split four ways, with `PageSections` threading props it never read.

- **`BacklinksSection.row-ops.ts`** — `createBacklinkRowOps`: every mutation a row can perform,
  and all of the section's state (editing, pending, the sheet's row, the delete prompt), so the component carries no signals of its own — the arrangement `Journal.remind.ts` and `Journal.embeds.ts` already use.
  **Mobile has no chord surface at all**, so where the desktop binds a key, a long press offers a row in the same `<BlockContextMenu>` the outline uses:
  task state, create below / above, indent, outdent, both moves, delete, each routed to the source page.
  The list is `buildBacklinkContextActions` in `Journal.context-actions.ts`, beside the outline's,
  and the eight rows it draws take their label and glyph from the same `SHARED_ROWS` table the outline's sheet reads — two icon literals is how one action comes to look like two.
  **There is no mark-done row**, here or anywhere on this client: `Action::MarkDone` is `Missing` on mobile, and row one cycles task state through to DONE instead.
  `Capability::BacklinkInlineEdit` covers the other nine operations for exactly that reason — a capability row may not claim an operation this client has nowhere.
  Three decisions in it, each written down because the honest answer is not the obvious one:

  - **Indent / outdent / move ship with no nudge and no `enabled` guard.**
    They have no visible result on a flat row, which was weighed and accepted — the capability row says `Full` for this client, and the TUI's `Partial` on it is about a missing source-page gesture rather than about these three.
    The outline's builder hides a move at the ends by resolving a sibling index, and a backlink row has no sibling list on screen, so guessing that position is worse than offering both.
  - **Create below / above ship, and give up only the keyboard.**
    A new sibling cites nothing, so it is a backlink of no page, appears in no row here,
    and has no editor to focus — so these two rows are the only ones that omit `armsKeyboard`, which would otherwise raise the keyboard over a textarea that never mounts.
    The block is created anyway and the user is told where it went, because a declared limit beats a wrong target: the TUI hit the same wall from the other side and its re-entry parked the caret on the *citing* block.
    The sentence is `@outl/shared/backlink-edit`'s `backlinkCreatedElsewhere` and `Journal.backlinks.ts` passes it the open page id, because it returns `null` when the source page *is* the one on screen.
  - **Delete asks when the block has children**, through `<BlockDeleteConfirm />` — the same prompt and wording the outline raises (`JournalDeleteDialogs.tsx` owns both).
    The case is *stronger* here, because a backlink renders one flat block and the subtree about to be trashed is not on screen at all.

  **The card takes one write at a time**, and the guard is card-wide rather than per-row because the re-ask replaces every row.
  Every op is a cross-page mutation plus an `O(blocks-in-workspace)` scan, so the window is long enough for a second gesture to land in it —
  a tap back into a blurred-but-mounted editor, a double tap on a checkbox, a long press that was still reachable — and **none of these ops is idempotent from the user's seat**: a second toggle flips the task back.
  **A timeout is not the guard** and would be worse than none: fire one while the write still lands and the user retries, so the task flips twice.
  The row is dimmed and `aria-busy` while pending; only the **acting** row shows it, so a gesture refused on another row of the card is silently dropped — recorded, not claimed fixed.
  **The direction toggle is behind the same guard**, wrapped in this component rather than in the host: a reorder is a write too, and the host holds `onToggleOrder` but not `pending`.

- **`Journal.backlinks.ts`** — `createBacklinkCard`: the card's direction (issue #142) and what happens after a mutation through one of its rows.
  Both go through **`mutate`, never the resource's `refetch`** — a refetch flips `loading`, and the rule above hands `undefined` down while that is true, so it would unmount the card the user just typed into for the length of the scan.
  This is also where `backlinkCommitPlan` runs, read **before** `applyView` replaces `view()`: `applyView` happens only when the mutated page *is* the page on screen (a page can cite itself), and the re-ask is unconditional.
  **`committed` resolves only once the fresh rows are in**, and the section awaits it before closing its editor —
  close on the write's own reply and the row renders the pre-edit tokens it still holds, so the user watches their edit revert and then reappear.

- **`Journal.boot.ts`** — `createJournalBoot`: the two workspace-open paths (`loadTodayWithRetry`, `pullAndReload`).
  Every await in here is capped. The open is the one the journal's skeleton waits on, so an uncapped call there is not a slow page, it is a page that never comes back.
- **`Journal.polls.ts`** — `installJournalPolls`: the connectivity listeners and every ticker (peers 3s, reminders 30s).
  Armed in the body so `onCleanup` owns the right scope, started only after the boot load settles.
- **`Journal.navigation.ts`** — `createNavigation(deps)`: `openView` (a page the user asked for) and the `resetForNavigation` behind it.
  Its own module because `applyView` cannot answer the question it asks. `applyView` compares slugs, which is the right net for "the page changed however it happened" and blind to the case that bit: **asking for the page you are already on.**
  Today's journal is usually the open page, so the Today button, the week strip's current day, a `[[ref]]` pointing back here and a deep link to this page all resolved to the same slug, and the zoom they were supposed to leave stayed up with nothing on screen moving.
  `Journal.day-nav.ts` and the deep-link listener take `openView` rather than `applyView` for that reason; a commit, a background poll, a peer reload and a capture into today keep `applyView` and keep the zoom.
- **`Journal.caret-nav.ts`** — `createCaretNav(deps)`: an arrow key that walks out of the block being edited, with a hardware keyboard.
  The decision is `@outl/shared/caret-nav`'s, so what is here is the ordering, and both halves of it have a failure mode.
  **The answer is synchronous and the work is not**: the textarea decides whether to swallow the key from the return value, while the commit is a backend round-trip — so the target is resolved up front and the commit runs behind it.
  **The commit lands before the neighbour is opened**, because `commitEdit` ends with `setEditingId(null)`; opening first would drop the user straight back out of Insert.
  A commit that fails moves nothing, since `commitEdit` keeps the editor open on the draft and walking away from that would strand text the user can no longer see.
  `commitEdit` **resolves `false`** on that path rather than rejecting, so the step reads its answer; catching a rejection alone let `startEdit` overwrite the unsaved draft.
  A key repeat landing while a step is in flight is swallowed, so a held arrow never commits the same draft twice.
  The caret is handed over through a pending offset applied in `onTextareaMount` rather than a prop threaded down three components: the element is what has to be written to, `Journal` already receives it there, and `parkCaret` survives the Solid `value=` re-binding that a raw `setSelectionRange` loses on iOS.
- **`Journal.remind.ts`** — `createRemindAuthoring(deps)`: which block the "Remind me…" sheet is open on, the rule it opened with, and the write back.
  It owns the signal, like `Journal.embeds.ts`, so the component holds no state for the feature.
  The order is the part with a failure mode.
  The current rule is read **before** the sheet opens and carried as *text*, because a rule that does not parse still has to reach the user: opening a picker on a default would show them a time their block never said.
- **`Journal.embeds.ts`** — `createEmbedResolver(slug)`: the `((blk-…))` / `!((blk-…))` and `![[page]]` resolves for the visible page, called from `applyView`.
  It owns both signals too, so the component holds no state for either.
  The skip rule is the part with a failure mode: a resolve is skipped only when the page **and** that kind's key both match the last one.
  Keyed on the slug alone, a newly typed ref keeps printing its raw handle until the user navigates away ([#16](https://github.com/outlmd/outl/issues/16)).
  **The two kinds keep separate keys and separate generation counters**, which is the same bug one door along.
  Share them and a newly typed `((blk-…))` cancels the page-embed reply already in flight; the key was recorded before the fetch, so nothing re-asks and the embed renders its literal `![[name]]` until the user navigates away.
  `createMapResolver` is the one implementation of that rule, generic over the kind, so there is no second copy to get wrong.

What is left is ~1,800 lines, almost all of it one `Journal()` function.
That is real debt, not a finished job: what is left is a state/effects question (edit lifecycle, sync signals, the native bridges, the sheet stack), not a "move these functions" question — phases 1-3 were the movable half — and it wants its own plan.
`.github/file-size-baseline.txt` holds the current number, and the CI ratchet means it can go down but not up.

## `BlockRow.tsx` is four files

Same treatment, same reason (846 lines).
`BlockRow` is the **recursion** now: it knows a row has an id and children, so it is where `(id)` is bound onto each callback before it goes down, and where the swipe affordance wraps the row.
The haptics stay on that side on purpose — a buzz is feedback for a *decision* (this delete happened, this TODO flipped), and the decision is made there; `BlockBody` only reports the gesture.

- **`BlockBody.tsx`** — the row's own content: the touch-gesture layer plus the read / edit branch, and the `rowPadLeft(depth)` the parent's guide line is positioned against.
  Three of its rules answer one question — *who already handled this touch?* — and they do not collapse into one.
  A press that starts on a ref must not arm the long-press timer.
  A click a child already consumed must not fall through to "start editing".
  And while a range selection is active, every touch means "extend the range to here".
- **`BlockMarkers.tsx`** — `CollapseTriangle` + `BulletOrCheckbox`.
  Together because they are one column: both reserve their slot even with nothing to draw, so a change to one's width is a change to the other's offset.
  Presentational only — neither knows what a tap means.
- **`BlockTextarea.tsx`** — `EditableTextarea`.
  Its own file because the rules in it are about iOS text input, not about outlines: `beforeinput` rather than `keydown` (WKWebView emits no reliable per-character key events) and `parkCaret` twice around every programmatic `value =`.
  **Its one `keydown` branch is the arrow keys**, and that is not a contradiction of the rule above: cross-block caret nav only ever fires from a *hardware* keyboard (an iPad, or an Android phone with one attached), which does emit reliable key events — the soft keyboard the `beforeinput` rules exist for has no arrow keys at all.
  The contract is **when the key is swallowed**: `onCaretExit` answers synchronously whether the step was taken, and only then does the handler `preventDefault`, so at the outline's edge the webview's own caret motion still happens.
  `@outl/shared/caret-nav` owns the decision and `Journal.caret-nav.ts` owns this client's walk; the desktop wraps the same pair.
  **The third rule is that nothing writes `ta.value` while an IME composes** ([#399](https://github.com/outlmd/outl/issues/399)).
  WebKit reports `selectionStart` at the start of the marked run, so pair completion read `[[` plus the pinyin as an unclosed ref and appended a second `]]`.
  Writing a different string then commits that run as plain text, which is how `nihao` typed inside `[[]]` became `[[]]nihao]]niha]]nih]]ni]]n]]`.
  `BlockTextarea.test.tsx` pins all three handlers, including the one that needs no guard, and the handlers carry the reasoning and what the guard costs.

**A prop this component accepts must reach the recursive call, and `BlockRow.recursion.test.ts` is what makes that fail.**
Two were declared and never forwarded, and both were invisible: optional props, so omitting them compiles.
`BlockRow` withheld `onSetProperty` from `<BlockBody />`, so tapping a `key:: value` chip edited nothing on **every** block; and the recursive `<BlockRow />` dropped `onSetProperty` *and* `onPasteMarkdown`, so a nested block fell back to the browser's literal splice — pasting a spreadsheet range or a markdown table into anything but a root block produced a wall of tabs instead of a table (found while closing [#329](https://github.com/outlmd/outl/issues/329)).
Both are fixed.
The test reads the JSX rather than rendering it (rendering needs haptics, a Tauri bridge and a WKWebView-shaped textarea — mocking all three would pin the mocks, and the defect is in the JSX), and a prop the children legitimately must not have goes in its `NOT_FORWARDED` table with a reason.
Same "declare the gap, don't discover it" shape as `outl_shortcuts::support` and `wire_types.rs`.

### A paste rewrites the block being edited

`pasteMarkdownAt` (in `Journal.block-ops.ts`) is three steps, and the last two are the ones easy to skip: flush the in-flight draft, fold the fresh view in, **then put the host block's new text back into the textarea**.

That third step is the fix for [issue #488](https://github.com/outlmd/outl/issues/488).
`draft` is seeded once by `startEdit`, so stopping at the view left the editor on the pre-paste string and `commitEdit` wrote it back over the paste on the next blur — a link copied out of an email gone from the block and from the `.md`, silently.
The text and the caret come off `PasteReply` (`pasteEditorUpdate`, `@outl/shared/paste`), never recomputed as "where I pasted plus what I sent": the backend built the string and may have inserted a separating space.
Writing it uses the `spliceText` + double `parkCaret` pattern `lib/textarea.ts` owns, and bails when `editingId()` moved during the round-trip.
Regression net: `Journal.block-ops.test.ts` → "pasteMarkdownAt — #488".
