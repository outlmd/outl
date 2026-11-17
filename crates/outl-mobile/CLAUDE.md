# CLAUDE.md — outl-mobile

Tauri 2 mobile client (iOS first, Android later).
Solid.js + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation is delegated to `outl-actions`.

## Capture from outside the app (RFC 0363)

Seven entry points on two platforms, and none of them touches the workspace.
Each writes one `<ULID>.json` into an inbox, and `drain_capture_inbox` (`outl-tauri-shared/src/capture_inbox.rs` over `outl_actions::capture`) is the **only** path from there into the workspace — root invariant 7 holds because every capture becomes ops at that one place.

| Platform | Entry points | Who writes the file |
|---|---|---|
| iOS | widget, Control Center "Quick Add" control (iOS 18+), "Add to today" App Intent, share extension | `OutlKit/Capture/CaptureInbox.swift`, into the App Group container |
| Android | share target, home screen widget, quick settings tile | `outl_actions::capture::write_capture` through JNI, into `filesDir/captures` |

**The platforms differ on purpose, in exactly one place: who owns the file format.**
An iOS widget and share extension are separate processes under a memory ceiling this library would not fit, so a small Swift writer owns it there, and the Rust reader's field names are spelled out on both sides.
Android's components are this app, in this process, with the library already loaded — so Kotlin holds neither the format nor the "a share is one line" rule (`shared_line`), only the folder and the strings.
`src-tauri/src/capture_jni.rs` is that boundary.

The widget, the iOS 18 control and the tile write nothing at all: they open `outl://capture`, which is why they can ship before an inbox exists (RFC 0363 step 1).
On Android that needs the `outl` scheme in `AndroidManifest.xml` — without it they open nothing.

Targets, signing and what needs a Mac: `IOS-EXTENSIONS.md`.
Android components and why they need no "a capture arrived" notification: [`docs/android-platform.md`](../../docs/android-platform.md) → Capture from outside the app.

## `Journal.tsx` is being broken up

1,804 lines in one component, still 3x the `file-size-guard.sh` ceiling.
The chrome came out first (`JournalChrome.tsx`), then the delete prompts (`JournalDeleteDialogs.tsx`) and the toolbar dispatch (`Journal.toolbar-dispatch.ts`); [issue 265](https://github.com/outlmd/outl/issues/265) phases 1-3 took the block ops, the selection state machine and the platform listeners out.
Day navigation and wake-to-today live in `Journal.day-nav.ts` (the wake *rule* is `@outl/shared/journal`'s `shouldWakeToToday`, shared with the desktop's day clock; this file owns the navigation around it), quick capture in `Journal.capture.ts` (`outl://capture` opens its sheet through `openCapture` in `Journal.listeners.ts`), and the empty states and zoom bar in `JournalEmpty.tsx` and `JournalZoomBar.tsx`.
The foreground pull and the capture inbox drain come from `@outl/shared/capture` (`installForegroundRefresh`), which the desktop uses too.
The two workspace-open paths came out last, into `Journal.boot.ts`, together with every ticker in `Journal.polls.ts` — they went together because both open paths hold a user-visible gate (`loaded` gates the skeleton, `syncing` gates both refresh affordances), and a gate set on one path and cleared on another is how a stall becomes permanent.
The remaining seams are in that issue, and the props rule an extraction must not break is in `outl-frontend-shared/CLAUDE.md`.

## Layering

```text
outl-core                    (CRDT, op log, storage trait)
outl-md                      (.md parse/render, sidecar)
outl-actions                 (workspace operations + SyncEngine, shared with TUI)
   ↑
outl-mobile (this crate)
   ├── src-tauri/src/
   │   ├── lib.rs                  (mod decls + run() + invoke_handler)
   │   ├── state.rs                (AppState + AppHost impl (incl. ProjectionWriter); wire DTOs re-exported)
   │   ├── workspace_open.rs       (boot orchestration over outl_tauri_shared::workspace_open primitives)
   │   ├── workspace_picker.rs     (set_workspace — folder choice + persistence; native picker deferred)
   │   ├── iroh_sync.rs            (wire_iroh_transport — boot the P2P transport, register the bg-sync handle)
   │   ├── bg_sync.rs              (one forced-sync core + per-platform exports: iOS C ABI, Android JNI)
   │   ├── android_jni.rs          (Android-only: primes rustls-platform-verifier before iroh's first QUIC connect; NOT ndk_context — that one is tao's, see below)
   │   ├── ios_bonjour.rs          (iOS-only: LAN peer discovery bridge to OutlBonjour.swift, over the outl_sync_iroh::lan seam)
   │   ├── plugin_service.rs       (PluginService + dedicated plugin thread — Boa Context is !Send, so it can't live in AppState)
   │   └── commands/               (Tauri command surface — one `outl_tauri_shared::*_commands!` invocation per module; see outl-tauri-shared/CLAUDE.md → "One command surface, not two")
   │       ├── mod.rs
   │       ├── workspace.rs        (workspace_stats, reload_workspace)
   │       ├── page.rs             (list_all_pages / search_pages / search_persons / outl_emoji_search / open_* / *_day / resolve_ref / legacy compat shims)
   │       ├── block.rs            (create / edit / toggle_todo / toggle_quote / delete / indent / outdent / move_* / set_collapsed / paste_markdown_at / copy_markdown)
   │       ├── peers.rs            (outl_peer_list / outl_peer_remove — read/edit <workspace>/.outl/peers.json, no workspace lock)
   │       ├── plugin.rs           (plugin_list / plugin_run / plugin_sync_hooks — thin shims over PluginService)
   │       ├── property.rs         (known_property_keys — the key catalogue the Properties sheet's chips come from)
   │       ├── exec.rs (run_code_block / run_auto_run_blocks / resolve_embeds)
   │       ├── shortcuts.rs (list_shortcut_bindings / list_action_support — the invariant-12 matrix)
   │       ├── timeline.rs (page_timeline, device_activity, export_markdown)
   │       └── theme.rs (list_themes / get_theme / get_theme_config)
   ├── gen/apple/.../main.mm       (NSMetadataQuery + NSFileCoordinator iCloud watcher)
   ├── gen/apple/.../OutlBackgroundRefresh.swift  (BGTaskScheduler windows + the beginBackgroundTask flush)
   ├── gen/apple/.../OutlBonjour.swift            (NetService advertise + NetServiceBrowser resolve, the iOS mDNS path)
   ├── gen/android/.../MainActivity.kt            (load check, then NativeSetup.install + OutlBackgroundSync.install, before Tauri boots)
   ├── gen/apple/.../OutlSafeArea.swift           (measures UIKit safeAreaInsets, pushes them to the page; see "Safe areas")
   ├── gen/android/.../OutlSafeArea.kt            (system bars + cutout from WindowInsets, pushed to the page)
   ├── gen/android/.../OutlNativeLibrary.kt       (the one "did the .so load?" answer; logs ABI + page size)
   ├── gen/android/.../NativeLoadErrorActivity.kt (plain error screen instead of a crash when the .so will not load)
   ├── gen/android/.../NativeSync.kt              (external fun bindings for the bg_sync JNI symbols)
   ├── gen/android/.../OutlBackgroundSync.kt      (ProcessLifecycleOwner observer + the two WorkManager schedules)
   ├── gen/android/.../SyncWorker.kt              (drives one forced pass, then finishes)
   └── (frontend in ../src)        (Solid components, Tailwind, Tauri bridge)
```

The command bodies, wire DTOs, helpers, and the plugin thread live in **`crates/outl-tauri-shared/`** (see [its CLAUDE.md](../outl-tauri-shared/CLAUDE.md)).
This crate keeps only thin `#[tauri::command]` wrappers plus what is genuinely mobile-specific (`bg_sync.rs`, `workspace_picker.rs`, the iOS glue).
The one structural divergence — mobile's `storage_root: PathBuf` instead of desktop's `Arc<Mutex<Option<PathBuf>>>`, because a folder swap is a relaunch here — is absorbed by the shared `AppHost` / `StorageRootProvider` traits.
The wrapper files therefore read identically to desktop's.
A command both clients need gets its body in `outl-tauri-shared` and a wrapper + `invoke_handler!` entry in **both** clients — never in just one.

The op log backend is the shared `outl_core::storage::JsonlStorage`;
there is no `icloud_storage.rs`, and no iCloud-specific Rust at all: the ubiquity-container resolution that used to live in `icloud_path.rs` was removed along with the file (see "Change detection: the iroh signal" below).
The storage trait stays generic; the transport gets handled outside it.

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

- **`Journal.listeners.ts`** — the three listeners the journal installs, over four backend events (`deep-link://navigate`, the webview file drop, and `projection-write-failed` + `workspace-ready` together), plus `navigateDeepLink` which the cold-start path calls directly.
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
That opener is `openView` (`Journal.navigation.ts`), not `applyView` — see that module's entry above.
  **Nothing here draws until the scan answers for the page on screen** (`lateSectionVisible`, `@outl/shared/outline`).
  `Journal` is what makes that true: `createResource` keeps its last value through a source change, so the raw accessor left the page the user just left answering for the one they opened, while the card said "No backlinks yet" over a scan still running.

- **`Journal.boot.ts`** — `createJournalBoot`: the two workspace-open paths (`loadTodayWithRetry`, `pullAndReload`).
  Every await in here is capped. The open is the one the journal's skeleton waits on, so an uncapped call there is not a slow page, it is a page that never comes back.
- **`Journal.polls.ts`** — `installJournalPolls`: the connectivity listeners and every ticker (peers 3s, reminders 30s).
  Armed in the body so `onCleanup` owns the right scope, started only after the boot load settles.
- **`Journal.navigation.ts`** — `createNavigation(deps)`: `openView` (a page the user asked for) and the `resetForNavigation` behind it.
  Its own module because `applyView` cannot answer the question it asks. `applyView` compares slugs, which is the right net for "the page changed however it happened" and blind to the case that bit: **asking for the page you are already on.**
  Today's journal is usually the open page, so the Today button, the week strip's current day, a `[[ref]]` pointing back here and a deep link to this page all resolved to the same slug, and the zoom they were supposed to leave stayed up with nothing on screen moving.
  `Journal.day-nav.ts` and the deep-link listener take `openView` rather than `applyView` for that reason; a commit, a background poll, a peer reload and a capture into today keep `applyView` and keep the zoom.
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

## What a launch waits for

**The skeleton is replaced by today's journal, read off disk.**
`Journal.tsx` subscribes to `onPagePreview` synchronously at mount (the event is emitted before the op log is opened) and renders `<PreviewOutline />` in the skeleton's place.
The subscription is deliberately **unguarded**: the render is the only reader of that signal and already sits inside the load gate, so a preview arriving late is simply never shown — and reading the gate out there would put a read outside the outline section, which `Journal.load-gate.test.ts` pins against.
The skeleton stays for a launch with nothing to preview (a fresh install, a day with no journal yet).

The splash is dismissed by the app, not by a timer.
The CSS version cost a **620 ms floor on every launch** (a 380 ms delay plus a 240 ms fade), paid whether the journal was ready in 180 ms or in four seconds, and it could not be shortened without risking the opposite failure.
`lib/splash.ts`'s `dismissBootSplash` starts the fade when the first view is applied, **including on failure** (that state carries a Retry button, and a splash over it is a dead end) and from `App.tsx` on the first-run path, which has no journal to wait for.
`index.html` keeps a 4 s failsafe animation as a backstop, never as the mechanism.

`Journal.boot.ts` waits for the workspace instead of retrying it: `waitWorkspaceReady` replaced 50 attempts at 200 ms of backoff, so the load resolves when the opener publishes rather than on the next tick of its own interval.
A rejection there means an older backend, and the open below it still answers `workspace_loading` while the opener runs.

The orphan sweep, the snapshot refresh and **the signal that lets iroh bind** are registered on `outl_tauri_shared::FirstPaintGate` and released by the frontend's `first_content` mark.
`run()` arms the gate's 6 s deadline, which is what keeps "deferred" from becoming "never" on a webview that fails to report.

**The `open_done` channel is not replaced by the gate.**
It is a memory gate rather than a readiness one (see "The workspace open and the first sync must not run at the same time" below) and it stays a channel because a channel cannot be missed.
What changed is that the send now happens from inside the gate, so the first dial does not compete with the first paint either.

Why each job waits, and the budgets: [`docs/boot-performance.md`](../../docs/boot-performance.md).

## The keyboard has to come up inside the tap

**A handler that creates a block from a tap taken with the keyboard down calls `armKeyboard()` before its first `await`** — WKWebView raises the keyboard only for a `.focus()` inside the tap's gesture window, and a create is two awaits deep.
Focus it after them and you get a genuinely focused textarea with no keyboard, which reads as "the + button doesn't let me type" and which nothing in jsdom can see.
`lib/keyboard-keeper.ts` is the one owner; `Journal.new-block.ts` holds the two handlers that need it and carries anything typed during the round-trip into the new block's draft.
A deferred tap is not a tap, so a `BlockContextMenu` action that ends in a text field declares `armsKeyboard: true` and the menu arms before it defers.
The mechanism, the per-path table and why the focus test is only a **proxy** (a device is what proves a keyboard): [`docs/mobile-ux.md` → Making a block and typing in it](../../docs/mobile-ux.md#making-a-block-and-typing-in-it-the-keyboard-handoff).

## Storage is a chosen folder, not forced iCloud (Fase 2)

**The workspace root is a folder the user picks.**
It may live anywhere — the app's local data dir (the default), the Files app, or inside an iCloud container — and **iroh P2P is the primary sync**.
iCloud is now just *a place the folder might be*, never a hard dependency.
A fresh install works with **zero iCloud**.

Boot resolution (`workspace_open::resolve_storage_root`):

1. The persisted `WorkspaceCfg.last` path (from `outl-config`), when present and usable — survives restarts.
2. Else the app-local default `<app-data-dir>/outl/` — synced by iroh, no iCloud.

The old behaviour (force `<ubiquity-container>/Documents/`, fall back to local only if iCloud was unavailable) is gone.
iCloud is reachable only as a path the user picks through the OS file picker — there is no `icloud_workspace_root()` / `pick_in_icloud` in the Rust path any more, and nothing in this crate resolves a ubiquity container.

**Folder selection.**
`workspace_picker.rs` owns the choice.

- `set_workspace(path)` validates, creates the dir, persists `WorkspaceCfg.last`, and emits `workspace-reopen-required`.
  The reopen is **boot-read** today (next launch picks up `last`); a runtime swap would need `AppState.storage_root` to become an `Arc<Mutex<Option<PathBuf>>>` plus an iroh rebind, deliberately deferred.
  Frontend wrapper: `setWorkspace(path) → Promise<void>` in `src/lib/api.ts`.
  No caller wires it yet — the arbitrary-folder native picker (`UIDocumentPickerViewController` + security-scoped bookmark) is deferred, so the local default is the only root a fresh install opens.

> **Registration note:** `workspace_picker::set_workspace` is registered in `lib.rs`'s `invoke_handler!` list. There is no `pick_in_icloud` any more (see the iCloud note above).

## First-run onboarding

`components/Onboarding.tsx` is the first-run flow.
`App.tsx` gates it on a **per-install `localStorage` flag** (`outl.onboarded`) — pure UI state, **never** an Op (it must not converge across devices; each device onboards once).
Mobile has no "is a workspace chosen?" backend gate (a fresh install always resolves *a* root — the local default), so this flag is the only signal that distinguishes a brand-new install from a returning one.

Two honest steps, no filler:

1. **Storage** — "Keep on this device" (the local default, recommended; just advances, no `setWorkspace` call) vs "Store in iCloud" (`setWorkspace(pickInICloud())`).
   The iCloud option is **hidden** when `pickInICloud()` returns `null` (device not signed in) — never a dead button.
   Because `set_workspace` is boot-read, choosing iCloud shows a one-line "active after you restart" note instead of pretending the swap is instant.
   Arbitrary-folder picking stays deferred (native picker), so these two are the only choices today.
2. **Sync (optional)** — the shared `SYNC_STEP` copy (`@outl/shared/onboarding`) + a button that opens the existing `<DevicesSheet />` (set its `open` prop — internals untouched).
   Fully skippable.

The onboarding **copy** lives in `@outl/shared/onboarding` (identical to desktop); the bottom-sheet chrome + haptics stay here.

**A third first-run surface sits after both steps**: `<GuideSheet />`, which names this client's own chrome — the bottom bar, the `+`, the week strip, the long-press ([#574](https://github.com/outlmd/outl/issues/574)).
It is not the 30-second outline tour (`firstRunTips`), which only ever draws on an empty page and names no buttons, so it carries its own dismissal key (`CHROME_TOUR_KEY`) rather than sharing `TIPS_KEY`.
Sharing one would hide it from the user whose journal already has content, which is exactly who needs it.
The rows are `lib/chrome-tour.ts` (mobile-only copy: every one names a control only this client has); the shared half is the "show it once, then remember" pair, which never becomes an Op.
Pairing is **not** reimplemented — `Onboarding` opens the real `<DevicesSheet />`.

**DEFERRED — native folder picker + security-scoped bookmark.**
The native `UIDocumentPickerViewController` (folder mode) bridge is **not** implemented (and is not faked).
Two real blockers: Tauri 2's iOS folder picker is incomplete (tauri-apps/plugins-workspace#3030), and a folder *outside* the app sandbox needs an `NSURL` security-scoped **bookmark** to be reopenable across launches.
Storing just the string path in `WorkspaceCfg.last` only works for the sandbox and the local default.
The follow-up adds an `objc2` bridge that presents the picker, serialises a bookmark, persists it next to `actor`, and resolves it on boot before `resolve_storage_root`.
Until then, `set_workspace` works for any path the frontend can already reach without a scoped bookmark, and the local default is the only root a fresh install opens.

## Change detection: the iroh signal

Storage is a **local folder synced by iroh** (no iCloud — the Rust side was ripped out: `icloud_path.rs` deleted, `storage_is_icloud`/`pick_in_icloud`/`icloud_workspace_root` removed).
The transport fires a reload signal whenever it writes peer ops; `iroh_sync.rs` bridges it to the `workspace-ready` Tauri event.
There is no filesystem watcher in the Rust path.

**DEFERRED — native iCloud cleanup.**
The iOS-native `OutlOpsWatcher.swift` (`NSMetadataQuery` + `NSFileCoordinator`), the iCloud container entitlements, and the `Info.plist`/`pbxproj` references are still present from before the Rust teardown.
Because the chosen folder is now always local, the watcher's `NSMetadataQueryUbiquitousDocumentsScope` query matches nothing and stays **dormant** — it does nothing and breaks nothing.
Removing it (watcher → no-op, strip the entitlements + plist keys) is a follow-up that touches code-signing, so it must be validated with a device build, not done blind.

## LAN peer discovery (mDNS)

Both platforms enter through `outl-sync-iroh`'s `bind::attach_mdns`, and *how* they discover diverges completely: Android joins the multicast group at socket level and needs a held `WifiManager.MulticastLock`; iOS cannot, so it browses through `mDNSResponder` (`OutlBonjour.swift`) and needs no entitlement.

The seam is `outl_sync_iroh::lan` — plain Rust, because that crate is `#![forbid(unsafe_code)]`, so the FFI lives here in `ios_bonjour.rs`.

Full reasoning, the two wire constants both paths must agree on, and the permission traps: [`docs/ios-platform.md`](../../docs/ios-platform.md#lan-peer-discovery-bonjour-and-why-not-multicast) and [`docs/android-platform.md`](../../docs/android-platform.md#mdns-peer-discovery-the-permission-is-not-the-whole-story).

## Android JNI bootstrap: prime the verifier, never `ndk_context`

`android_jni.rs` primes `rustls-platform-verifier`'s JVM handle from `MainActivity.onCreate`, before `super.onCreate` boots Tauri and therefore before iroh's first QUIC connection.
That global is ours because nothing else primes it.

**`ndk_context` is not ours**, though the system-DNS reader goes through it.
`tao`'s `ndk_glue::onCreate` seeds it inside `Rust_onCreate`, and `ndk_context::initialize_android_context` asserts it is called exactly once — so seeding it here first did not prevent tao's call, it made tao's call abort the process.
Every Android launch died in `panic_cannot_unwind`, on every device, on a clean install, hidden behind the 16 KB page-size bug sitting in front of it.
`outl-mobile` must not name `ndk-context` in its own `Cargo.toml` again; it stays in the graph transitively, for tao to write and `iroh-dns` to read.

The stack trace, the reasoning and the rule it generalises to: [`docs/android-platform.md` → `ndk_context` belongs to `tao`](../../docs/android-platform.md#ndk_context-belongs-to-tao-and-seeding-it-here-aborted-every-launch).

## Background sync (iOS + Android)

Neither OS lets a backgrounded app keep syncing, and an iroh delta torn down mid-flight makes the peer log `peer did not confirm durable ingest`.
So each platform drives a **forced** pass inside whatever window the OS grants.

**`bg_sync.rs` is the one owner of that pass, for both.**
A platform scheduler calls into it; it never reimplements the pass.

The schedulers, their windows, and what can only be verified on a device: [`docs/ios-platform.md`](../../docs/ios-platform.md#background-sync-ios) and [`docs/android-platform.md`](../../docs/android-platform.md#background-sync-android).

## Hard rule

**This crate adds no business logic.**
If a Tauri command does something that involves the workspace shape (edit, move, todo, journal render), it delegates to `outl-actions`.
If you find yourself writing a tree walk or an op-generating helper inside `lib.rs`, stop — move it to `outl-actions` instead.
The TUI will need it too.

The same rule extends to the **Solid frontend** (`src/`).
Before adding a helper that walks blocks, normalises text, or maps a cursor across `\n`, check:

1. **`@outl/shared`** (`crates/outl-frontend-shared/`) — the cross-client TS lib already owns `<MarkdownInline />`,
   `looksLikeOutline`,
   `utf16OffsetToCharOffset`,
   the autocomplete helpers (`detectRefContext`, `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion`),
   every shared DTO (`@outl/shared/api/types`),
   and the `invoke()` wrappers for the Tauri commands every client calls (`@outl/shared/api/commands`).
   If you find yourself reimplementing one of these in `src/lib/`, stop — the desktop client will need the identical behaviour, and a parallel TS copy is exactly the drift we paid to delete.
2. **`outl-md` / `outl-actions` / `outl-core`** — the Rust side likely already exposes the data through a Tauri command (or could with a tiny addition).

Only write a helper directly under `outl-mobile/src/lib/` when it's genuinely mobile-specific (touch gestures, iOS UIKit bridges, haptics, viewport math).

Workspace-level policy: [`CLAUDE.md`](../../CLAUDE.md#reuse-first).
Frontend-specific policy: [`outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md).

What this crate **does** own:

- iCloud Ubiquity Container resolution and the `Storage` impl on top.
- Per-device actor id persistence (`<sandbox>/actor`).
- Tauri command surface (argument parsing, error mapping).
- Solid frontend that consumes the commands.

## "This page isn't syncing" banner

`<PageAheadOfLogBanner client="mobile" />` (from `@outl/shared/warnings`) renders above the outline when `PageView.md_ahead_of_log` comes back set.
That means the page's `.md` holds lines the op log never recorded, so outl refuses to overwrite it and the page has stopped converging with the user's other devices.
The copy is owned by `@outl/shared/warnings::aheadOfLogNotice`, never written here; `client="mobile"` is what makes it say "open this workspace on your computer" instead of naming a terminal command, because **there is no `outl` binary on iOS**.
The flag is held in `Journal.tsx`'s own `aheadOfLog` signal, **not** read off `view()`: only the open commands carry it, so an edit commit's reply would otherwise clear the banner on the user's first edit — the very action it warns against.
It is cleared by a reply carrying `md_ahead_of_log_checked` with no notice (the next open / refresh once the page is healthy again), so the banner can't outlive the condition — same rule as desktop.
Convention: [`docs/clients.md` → Surfacing a page that stopped syncing](../../docs/clients.md#surfacing-a-page-that-stopped-syncing).

## Page switcher: rename and merge (RFC 0557)

A long-press on a **page** row (never a journal) opens the same menu component the block long-press uses, with three rows: **Rename page...**, **Merge into...**, **Delete page**.
`PageSwitcher.tsx` keeps the search list and the wiring; everything one page can be asked to do lives in `PageSwitcher.page-actions.tsx` (`createPageActions`, the long-press recogniser, `<MergeBanner />`, `<PageActionOverlays />`), split out when the switcher would otherwise have crossed the 600-line ratchet.

- **The flow is not written here.** `renameOrMerge` / `mergeInto` and every sentence (`nameTakenMessage`, `mergeConfirmMessage`, `renameSummary`, `mergeSummary`) come from `@outl/shared/pages/rename-merge`, so the desktop runs the same steps and says the same words.
  This crate owns only the chrome: `PageNameSheet.tsx` (a prefilled one-line sheet, 48pt targets) for the name and `ConfirmDialog` for the question.
- **A merge is never reached without a confirmation.** "Merge into..." turns the switcher's own list into a target picker (pages only, the source left out, no pin toggles, no long-press) and a tap on a row asks first.
  Cancelling that question stays in the picker so another target can be chosen; the banner's Cancel leaves it.
  A rename onto a taken name asks the same question, and any answer but "Merge" writes nothing.
- **On success** the switcher navigates to the resulting page through `onPick` (delete does the same) and hands the summary to `onNotice`, which `Journal.tsx` routes into its toast.
  A summary with several lines (a merge with property conflicts) makes `Toast.tsx` show every line and stay 12 s, because the conflict line is the one the user has to read.
- **The `+` beside the search box starts a page**, name sheet → `openRef`, which is also the only reason `PageNameSheet` grew a `submitUnchanged` prop (a rename refuses a no-op name; a create prefilled from the search box must not).
  Before it, a non-journal page could only be started by typing `[[` into a block ([#573](https://github.com/outlmd/outl/issues/573)) — the mechanism was fine, the discoverability was the bug.
  `openRef` decides what a name means, so an existing name **opens** rather than duplicating; see [`docs/mobile-ux.md`](../../docs/mobile-ux.md#search-sheet--starting-a-page-that-is-not-a-daily-note).
- Not done on mobile: a keyboard shortcut (there is no chord surface) and the optional alias-conflict warning.
- Pinned by `PageSwitcher.page-actions.test.tsx` (the shared flow runs for real over stubbed `api/page-merge` commands) and `sheets.bounded.test.ts` (the name sheet declares its bound).

## Safe areas: the native shell measures, the page applies

`env(safe-area-inset-*)` is not trustworthy in either webview here: both shells are edge-to-edge (full-screen `WKWebView`; `enableEdgeToEdge()` in `MainActivity`), and with the inset reading 0 the header's `max(env(...), 12px)` fell to its floor and drew under the status bar.
So there is one source: `OutlSafeArea.swift` (iOS, a probe view's `safeAreaInsetsDidChange`) and `OutlSafeArea.kt` (Android, the `WindowInsets` listener, system bars plus display cutout, IME excluded) push CSS-pixel values into the page, and `src/lib/safe-area.ts` writes them to `--safe-area-inset-*` on `:root`, installed once from `index.tsx`.
`styles.css` declares those properties from `env(..., 0px)` as the fallback.
**A component reads `var(--safe-area-inset-*)`, never `env()`**; `safe-area.css.test.ts` fails on a stray `env(safe-area-inset` and on a fallback without a length.
Load order is covered both ways: native pushes on change and when the webview appears, the page pulls (iOS `outlSafeArea` message handler, Android `window.OutlSafeArea.get()`).
Only a device can show the numbers, so check a notched iPhone in both orientations and an Android phone with gesture and three-button navigation before calling it done.

## Per-feature behaviour lives in `docs/mobile-ux.md`

Nine sections moved out: how a ref that does not exist yet is opened, the page switcher's long-press delete, external links, blockquote chrome, zoom / focus, paste from external apps, the keyboard accessory bar, code execution, and template insertion.

They are reference — you look one up when you touch that feature — and this file is loaded into context on every session in this crate, where the budget belongs to the rules you must not break.
[`docs/mobile-ux.md`](../../docs/mobile-ux.md) is their owner now.

## Properties are fields, not tags

A pill is a **tag**: a flat label you tap to filter.
A `key:: value` property is a **field**, a named attribute with a value, and this client drew one as the other — so two different things read as the same thing while behaving differently, and the tap a pill promises was not the tap it performed.

The outline now draws **one attribute line** per block: key dim, value a step darker, `·` between, truncated, never more than a line, and past two fields the rest is `+N more`.
Both the block row (`BlockBody.tsx`) and the page strip (`Journal.tsx`) are the same `<BlockProperties />` now — the strip was a second hand-written copy, which is how the same dead `text-[11px]` came to be written twice.
`@outl/shared/markdown` owns the shape; what is left here is where the line sits and what a tap means.

**Colour is spent only on a field that acts on the user.**
`remind::` gets the accent and a bell because it will interrupt you; `auto-run::` runs code and stays quiet, because it does not. That criterion is *interruption*, not importance, and it is what stops the line from becoming decoration again.

**Editing left the outline.**
The line is one tap target that opens `<PropertiesSheet />`, which is a two-column field list (`PropertyFieldList.tsx`) in the shape of iOS Settings or Notion's property panel. A row that looks like a form row opens a form; a row that looked like a button did not.

An importer's stamps (`created`, `edited`, `roam-uid`) render nowhere in an outline and are still listed in the sheet, dimmer and sorted last — you have to see one to delete it. The policy is `outl_actions::property::is_import_stamp`, mirrored in TS.

## Properties (`key:: value`) — the sheet (issue #13)

Creating a property was impossible in either GUI client: the chips
rendered what a block already carried and edited a value in place, so
the *first* one had to come from the TUI or a hand-edited `.md`. Page
properties (`icon::`, `type::`) were not even visible.

**`components/PropertiesSheet.tsx`** is mobile's answer. The block
long-press menu's "Properties…" action opens it on that block; the
page's own chips (rendered under the header in `Journal.tsx`) open it
on the page, and a Block/Page switch inside crosses between the two.

Two design facts, both measured on a real 66k-block graph, decide the
shape — and both are the reason this is a sheet and not a `+` on the
chip row:

1. **Keys repeat.** A graph reuses about a dozen (`icon`, `related`,
   `status`, `oura-date`). So the Add step paints the workspace's own
   keys as **tappable chips**, ranked by `known_property_keys`, with
   "Other…" as the keyboard escape. Two taps, nothing typed — that is
   ~87% of the value of this feature on a phone.
2. **87% of values are a `[[page]]` or a `#tag`.** Typing `[[` in the
   value field opens the page picker (`searchPages` + the shared
   `detectRefContext` / `applySuggestion` / `refReplacement` — the
   accept rule is **not** re-implemented), and a "Link a page…" button
   types the `[[` for you.

Deleting is explicit in two shapes: swipe a row (`<SwipeRow>`) and a
Delete button inside the value editor. Both write an **empty value**,
which is how `set_property` spells "remove" — the user never has to
discover that.

Wire commands, both thin wrappers over shared bodies:

- `known_property_keys` (`commands/property.rs` → the catalogue; the
  ranking itself is `outl_actions::known_keys`, one owner, three
  clients).
- `set_page_property` (`commands/reminders.rs`, next to the existing
  `set_block_property` — same shared module owns both bodies). It
  refuses `page-slug` / `page-kind`: those *are* the page's identity,
  and renaming is `page_rename`, which moves the projection too.

`PageView.page_properties` is what makes the page side possible at all.

Mobile-only pieces, and why they are not in `@outl/shared` yet:
`lib/properties.ts` (`normalizeKey` — strips the `::` a user copies —
plus `suggestedKeys` / `editableProperties`) and the sheet itself are
phone-keyboard answers; the desktop's affordance for the same
catalogue is inline completion with no cap and no "Other…" escape.
`knownPropertyKeys` / `setPageProperty` live in
`@outl/shared/api/commands` beside `setBlockProperty` — both clients
register those commands, so the wrappers have one owner; `lib/api.ts`
only re-exports them so existing imports keep resolving.
The sheet's deny-list delegates its bookkeeping half to
`@outl/shared/markdown::isInternalKey` (the same predicate
`propertyChips` hides those keys with) and keeps only the
page-identity pair (`page-slug` / `page-kind`) local.

Covered by `PropertiesSheet.test.tsx` (chips exclude keys already set,
two-tap add, `::` normalisation, edit, both delete affordances, page
scope, the `[[` picker) and `lib/properties.test.ts`.

## Settings (`SettingsSheet.tsx`) — the first preferences surface

The header capsule's gear opens `components/SettingsSheet.tsx`.
It is the client's **only** preferences screen: every other sheet here (`DevicesSheet`, `TemplateSheet`, `RemindersSheet`, `PropertiesSheet`) shows workspace content, which is why the toolbar lock ([#271](https://github.com/outlmd/outl/issues/271)) had nowhere to live before it.
Its first two rows are the exception, and both are navigation rather than preferences.
**Devices and sync** hands over to `<DevicesSheet />`, closing this sheet first since this one is `z-[55]` and that one `z-50`.
**What's what** re-opens `<GuideSheet />`, the chrome walkthrough that otherwise only appears once per device ([#574](https://github.com/outlmd/outl/issues/574)).
It is here because pairing otherwise had no name anywhere on the phone — the sync dot's press-and-hold is a gesture nothing announces, and a user looking for P2P looks where the desktop keeps it, in Settings.
The gesture stays as the shortcut.

Two rules govern what may go in it:

- **Per-device UI state, not workspace data, unless the process itself has to read it.** Nothing here goes through the op log: two devices disagreeing about their own toolbar layout is not a conflict to reconcile (root `CLAUDE.md` invariant 7). The toolbar lock stays in `localStorage`. `editor.diagrams` is per-device too but lives in `~/.config/outl/config.toml`, so the desktop on this machine sees the same choice; the sheet reads and writes that key through `get_editor_diagrams` / `set_editor_diagrams`. A preference that *is* workspace data belongs in `outl.toml`, not here.
- **The setting's logic lives in `@outl/shared` or in a shared command, not in the sheet.** The toolbar lock is `@outl/shared/toolbar`'s `lock.ts`, the diagram switch is `@outl/shared/diagram`'s `diagramsMode`; this file only renders the row and calls them. A preference implemented inside the sheet is a preference the other bar can't honour.

**Crash reports goes further than either, because it is consent rather than a preference** (RFC 0377).
`localStorage` cannot hold it: `outl-telemetry` reads `[telemetry] enabled` out of the global config at **process start**, before a workspace or a webview exists.
It is a property of the person, which is why it belongs in a config file that follows them to the next device.
And the phone has to offer it at all: reporting is on by default and there is no `outl` binary on iOS, so this row is a mobile user's only opt-out (`Capability::CrashReportingOptOut`).
The writer is `outl_tauri_shared::commands::telemetry::set_telemetry_enabled`, the wrappers are `@outl/shared/api/telemetry`, and the words are `CRASH_REPORTS_SETTING` in `@outl/shared/onboarding` — identical to the desktop's, because the same claim goes to Apple and Google.
The switch moves only on the command's reply, like `RemindersSheet`'s; a read or write that fails says so in the row instead of falling back to the default, since the default is *on* and a guess would tell someone who opted out that they had not.
This row is the opt-out, not the disclosure: the strip that tells an unasking user at all is `<CrashReportsNotice />` from `@outl/shared/polish`, mounted by `App.tsx` **inside the onboarded branch** so it never stacks on the first-run flow, and lifted clear of `BottomBar` by `--outl-notice-offset` in this client's `styles.css` (RFC 0377 section 6).

**The sheet is mounted for the app's lifetime, not per open** — `<Show>` gates its markup, and `Journal` renders it unconditionally. So state it reads out of a store goes in a `createEffect` guarded on `props.open`, the way `TemplateSheet` and `RemindersSheet` already do. A plain `createSignal(readSomething())` in the body reads once, at app start, and then quietly shows a stale value forever.

**A preference this sheet writes has to be readable by whoever acts on it.** The toolbar lock's first version stored the counts it derives from in `UserDefaults` on the iOS side, where this sheet — which is web on both platforms — could not see them: Lock froze a cold-start row instead of the user's, and Reset was a no-op. Before adding a setting, check that the surface which consumes it reads the same store this sheet writes.

Toolbar behaviour it controls (lock, reset, and why the row now only reshuffles between editing sessions): [`docs/mobile-ux.md` → Keyboard accessory bar](../../docs/mobile-ux.md#keyboard-accessory-bar-android-web-bar--ios-native-bar).

## Reminders (`remind::`)

The bottom bar's Reminders tab opens `<RemindersSheet />` (the header has no bell any more), a block long-press authors a rule through `<RemindSheet />` → `set_block_remind`, and `Journal.tsx` polls `deliver_due_reminders` every 30s.
Sheet wiring and why both device-local settings live in the list sheet: [`docs/reminders.md`](../../docs/reminders.md#mobile-outl-mobile).

**Two rules, and neither is about the sheet's markup.**

- **The schedule is never computed in TS** — `groupReminders` / `formatNextFire` only format what `outl_actions::reminders` decided.
- **The rule text is never written in TS either.**
  `<RemindSheet />` reads a rule with `parseRemindRule` and writes one with `formatRemindRule`, two commands over `outl_md::remind`'s `parse_remind` / `format_remind` pair, twice per session rather than per keystroke.
  Authoring was a `window.prompt` for the grammar until [#572](https://github.com/outlmd/outl/issues/572).
  The sheet keeps the whole grammar reachable through its **Edit as text** field, which is what stops a picker from quietly narrowing what a user can say.
  `Journal.remind.ts` owns the target signal and the write, the way `Journal.embeds.ts` owns its own — the component holds no state for it.
  Per-control decisions (a repeat the list does not offer, a seeded `until`) are `lib/remind-fields.ts`, unit-tested.

### Actionable banners (mobile only)

A reminder banner carries **Snooze 1h** and **Done**, and a plain tap lands on the block rather than the journal.
Three pieces, and the split between them is the point:

- `commands/reminders.rs` owns the ids (`REMINDER_CATEGORY` / `ACTION_SNOOZE_1H` / `ACTION_DONE`), stamps the category onto every banner via `action_type_id`, and attaches `blockId` / `pageSlug` as extras so the handler knows the subject.
  `reminder_action_catalog` hands the same constants to the frontend, with a declared `kind` (`snooze` / `done`) so the handler dispatches on meaning instead of pattern-matching the id.
- `lib/reminder-actions.ts` is the decision table (which button → which call), pure and unit-tested, including the tap identifiers both platforms send and the "banner with no subject" case an upgrade can produce.
- `lib/reminder-notifications.ts` is the IPC + listener wiring, which can only run on a device.

**Why the frontend registers something Rust owns:** the plugin's `ActionType` / `Action` are `#[cfg(mobile)]` structs with private fields, no constructor and no `Deserialize`, so `Notification::register_action_types` is unreachable from a consumer crate.
`registerActionTypes()` in JS reaches the same command through IPC, so that is the only door.

**Never move this into `@outl/shared`.** The command, `registerActionTypes` and the `actionPerformed` event are mobile-only; the desktop plugin registers no such command and emits no such event, so a shared wrapper would hand the desktop an IPC call that fails.
The per-client verdict is declared once, in `outl_shortcuts::capability_support` (`Capability::ReminderNotificationActions`), and pinned here by `capability_parity.rs`.

## Plugins

JS plugins (`outl_plugins::PluginHost`) run on mobile; the design is the desktop's.
Read [`docs/plugins.md` → Desktop plugin surface](../../docs/plugins.md#desktop-plugin-surface) for the shared rationale (`!Send` Boa host on a dedicated thread, `PluginService` in `AppState`, re-projection via `apply_all_pages_md`).
Boa is pure-Rust (no JIT), so it ships under iOS's dynamic-code ban (same as `lang-js`).

**The one divergence from desktop:** mobile's `storage_root` is an owned `PathBuf` (folder swap is a relaunch), absorbed by the shared `StorageRootProvider` trait — a fixed root never triggers the "re-load on root swap" branch.
The host loads plugins once, lazily, from `<root>/.outl/plugins/` on the first request after the workspace opens (`ensure_loaded` + `mark_synced`).

Capabilities honored: `slash-command` + `op-hook` + `ui-render` + `toolbar-button` + `content-transformer:text` + `content-transformer:rich` (no `keybinding` — no chord surface on mobile).
Each must be declared in `client_capabilities()` (`plugin_service.rs`); the host gates contributions on the client∩plugin intersection.
Dropping `ToolbarButton` silently empties `toolbar_buttons("mobile")`; dropping either transformer cap silently filters `transformers()` (a custom-language fence then renders as plain code).
Tauri commands in `commands/plugin.rs` have the **identical shape to desktop** — the full command table is in [`docs/plugin-architecture.md`](../../docs/plugin-architecture.md#client-tauri-command-surface-desktop--mobile).

Op-hooks fire at a single post-mutation point: `Journal.tsx`'s `commitEdit` calls `pluginSyncHooks(pid)` after an edit lands.
One call dispatches every op since the last sweep, so it also catches structural ops (indent / move / delete).
The hook-driven `applyView` guards on `!editingId()` so it never resets the textarea.

Frontend: the plugin DTOs + wrappers (`pluginList` / `pluginToolbar` / `pluginRun` / `pluginSyncHooks`, …) live in `@outl/shared/api`.
The stacked-squares header glyph opens `components/PluginSheet.tsx` — a bottom sheet that lists + runs commands and pipes `notify` / errors to the toast.
Toolbar buttons are inline header glyphs.
`Journal.tsx` loads `pluginToolbar()` in `onMount` into a `toolbarButtons()` signal and renders one `<button>` per entry next to the sheet glyph.
`runToolbarButton` → `pluginRun(...)` reuses the sheet's toast / `showPluginViews` / `applyView` path.

### `ui-render` views (the confetti path)

A `ui-render` plugin emits HTML/JS via `ctx.ui.render(html)`; the core gates it onto `PluginRun::views`, propagated as `views` on both `PluginRunReply` (command path) and `PluginSyncReply` (`onOp` hook path).

`components/PluginViewOverlay.tsx` paints each in a **sandboxed, ephemeral `<iframe>`**.
The frame is `sandbox="allow-scripts"` **WITHOUT `allow-same-origin`** — load-bearing.
Plugin JS is untrusted; the missing flag forces an opaque origin so the frame can't reach the app DOM / Tauri bridge — the two flags together defeat the sandbox, so **never** add it.
Content is `srcdoc={html}` (no network), fullscreen, `pointer-events: none`, auto-removed after ~6s.

The overlay exposes an imperative `push(html)` via its `bind` prop.
`Journal.tsx` holds it as `pushPluginView` and feeds it from `showPluginViews(views)` at every source: `PluginSheet`'s `onViews`, `commitEdit`'s `pluginSyncHooks` reply, and `runToolbarButton`.
**End-to-end:** block → DONE → `commitEdit` → `plugin_sync_hooks` → `onOp` emits HTML → `showPluginViews` → iframe overlay.

The sandbox attrs + auto-removal are pinned by `PluginViewOverlay.test.ts`; `plugin_service.rs` unit-tests the host surface.
A real plugin load + iframe overlay only exercise under `cargo tauri ios dev`.

### Content transformers (custom-language fences)

A transformer claims a fence language and turns the body into a `{kind, content}` descriptor.
The registry + `(block id, body)` cache glue is shared with the desktop in `@outl/shared/plugins/transformer-registry` (see [`docs/plugins.md` → Desktop plugin surface](../../docs/plugins.md#desktop-plugin-surface)).
Mobile just wires it: `Journal.tsx`'s `onMount` calls `loadTransformers()`, and `BlockRow`'s fence branch renders `<PluginFence />` on a `transformerFor(lang)` match (else plain `<HighlightedCode />`).
`rich` output lands in an inline sandboxed `<iframe>` — `allow-scripts`, never `allow-same-origin`, same posture as `<PluginViewOverlay />`.

## Peer / device management (`outl_peer_list` / `outl_peer_remove`)

`commands/peers.rs` wraps the same three Tauri commands `docs/sync.md` § Managing peers documents (`outl_peer_list`, `outl_peer_remove`, `outl_sync_now`) over `<workspace>/.outl/peers.json` (`outl_sync_iroh::PeersStore`).
Mobile-specific: the peer list is per-**graph** (`outl_sync_iroh::workspace_peers_path`), the device `identity.key` is per-**install** ([`iroh_sync::iroh_dir`]).
Each command runs `migrate_global_peers_if_absent` first (legacy global `~/.outl/peers.json` copied in once).
These are the **only** commands that touch `peers.json` directly instead of the workspace lock — graph-scoped sync-transport state reads `storage_root` without going through `outl-actions`.

## Sync dot + refresh (iroh-driven)

The header `<SyncDot>` and `PullToRefresh` reflect and drive the **iroh P2P transport** (outl's default sync), not the iCloud-era `navigator.onLine` signal they started on.

- **Dot state.**
  The PRIMARY input is iroh peer health, polled via `peerStatus()` into a `peers` signal (`PeerStatusDto[] | null`, `null` until the first probe answers — the absence of a measurement, which `[]` is not).
  The poll runs every 3s, and after each `peer-ops-changed` (the native ops bridge) plus after a force-sync.
  It is **armed** in the component body but **started** at the end of `onMount`, after the boot load settles (`Journal.polls.ts`): probing during render put a full workspace load in front of the first paint.
  `peer_status` is `async` now, so that load is off the iOS main thread, but it still reopens the op log per call — deferring the first probe stays right.
  **The derivation is not written here**: `JournalChrome` hands the list, `navigator.onLine` and the two in-flight flags to `syncState` (`@outl/shared/peers`), which answers `syncing` / `checking` / `online` / `offline` for this client and the desktop alike, and `<SyncDot>` paints it.
  A pass in flight wins; no radio floors to offline; `null` is **checking** (dim), which is the bug the shared owner closed — this client used to start at `false` and claim offline on launch over a probe nobody had taken ([#575](https://github.com/outlmd/outl/issues/575)).
  `navigator.onLine` stays only as a secondary floor (truly no radio → orange regardless).
  Zero paired peers reads as offline — there's nothing to sync with.
- **Refresh.**
  There is no separate refresh button: the sync dot is the button. A tap fires `handleRefresh`, press-and-hold opens the devices sheet (`createLongPress`, the same recogniser the block row and title use) — a shortcut for the named row in Settings, never the only way in.
  Taps while a pass is in flight (`syncing` or `refreshing`) are dropped, and `handleRefresh` itself refuses to start a second pass, so the dot, the pull and the auto-sync can never stack dials.
  `handleRefresh` (the dot **and** `PullToRefresh`) calls `syncNow()` (force a P2P pull — dial every peer now instead of waiting for the 8s catch-up tick) THEN `reloadWorkspace()` (re-render with whatever landed).
  Both calls are wrapped in `withError` (toast on failure, never wedge the local reload), and the `syncing` spinner brackets the whole pass.
- **Auto-sync (no button).**
  `Journal.tsx` shares the refresh core as `pullAndReload()`, fired on `onMount`, on `visibilitychange` → visible (iOS froze JS in the background), and on the 3s poll tick.
  The mobile side initiating the dial is NAT-friendly — waiting for the desktop to reach an iPhone behind carrier NAT is not — so this is what makes a desktop edit show up without the user touching refresh.
  The `workspace-ready` reload skips while a block is being edited (guarded by `editingId()`) so it never resets the textarea mid-edit.
  It also routes through `pullAndReload` (not a raw `openJournalFor` + `applyView`) so it inherits its guards.
  **Reload-ordering guard:** each `pullAndReload` captures a monotonic `reloadGen` at entry and applies only when still the latest, so a slow reload can't flip the page back to an older state.
  (The split-content flicker's real cause was a backend duplicate-slug-root bug, fixed in `outl-actions::merge_duplicate_slug_roots`; this is the frontend belt-and-suspenders.)

`syncNow()`, `peersOnline()`, `syncState()` and `<SyncDot>` all live in `@outl/shared` so mobile and desktop derive the dot + drive the refresh identically.
There is no `SyncDot.tsx` in this crate any more; the sync button's `aria-label` names the action only, because the shared dot is the live region that announces the state.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

## Cross-runtime contracts (now in `@outl/shared`)

TS pieces mirroring a Rust canonical source (`looksLikeOutline`, `<MarkdownInline />`, `detectRefContext`, `autoPairBracket`, `utf16OffsetToCharOffset`, …) used to live as copies under `lib/`.
Extracted to `crates/outl-frontend-shared/` so mobile and desktop share one file.
Contract-by-contract table (moved out to keep this file under the per-crate size ceiling): [`docs/primitives-markdown.md` § 9](../../docs/primitives-markdown.md#9-cross-runtime-ts-contracts-outlshared).
**A new contract goes in `@outl/shared` from day one**, never under a client's own `src/lib/` first.

## Theming (RFC 0022)

`@outl/shared/theme::installTheme` fetches both pair sides via the shared `get_theme` command, holds both `Palette`s in memory, and repaints through `applyPaletteToRoot` on a `prefers-color-scheme` flip (no backend round-trip mid-repaint).
`App.tsx` reads `outl_config::ThemeCfg`; a fresh config already resolves to the brand pair `outl-light` / `outl`, while older configs with only `preset` keep that preset on both sides.
Field reference: [`docs/config.md` § `[theme]`](../../docs/config.md#theme).

**A text colour belongs on the element that holds the text, and `src/styles.css` must stay out of its way.**
An unlayered rule outranks every layered one regardless of specificity, and Tailwind's utilities live in `@layer utilities` — so the `button, input, textarea { color: inherit }` this file used to carry unlayered silently killed **every** `text-*` class on a `<button>`.
The week strip's selected day painted its accent tile and then rendered near-black text inside it, because the colour resolved to nothing and the spans inherited `fg` from `#root`; the same failure was live on every primary accent button here.
The `color` line is gone (Tailwind's preflight already sets it in `@layer base`, which beats the UA sheet and loses to a utility, which is the order you want).
`font-family` / `font-size` are still unlayered there and still beat `text-[16px]` on a button — if you need a button's font size to take, that is why.
Paint a fill with `bg-(--color-outl-accent)` and its text with `text-(--color-outl-accent-fg)`, never with `text-(--color-outl-bg)` (root `CLAUDE.md` invariant 13, and [`DESIGN.md`](../../DESIGN.md) owns the pairing).

**So the property line sizes the row, not the chip inside it.**
Both `key:: value` strips here carried a `text-[11px]` on a `<button>` that resolved to nothing: they painted at the body's 17px, centred (a `<button>` centres by UA default) and wrapping, so a Roam-imported block showed nine lines of grey chrome above one line of its own text.
The size moved to the container — where `inherit` resolves — and `property-line.test.ts` pins both call sites, asserting the unlayered rule is still in `styles.css` so the pin retires with the trap rather than outliving it.
**24 other `<button>` / `<input>` tags in this client still carry a dead `text-*`**, measured; clearing those means deleting the declaration and re-reading every sheet, which is the typography change the comment above defers.

## Logging (device console)

`run()` in `src-tauri/src/lib.rs` ignores `SIGPIPE`, then calls `outl_telemetry::init`, then installs a `tracing_subscriber` fmt subscriber.
The subscriber writes through `outl_telemetry::lossy_stderr` with `log_internal_errors(false)`, for the same reason as the desktop: a closed stderr must not become `failed printing to stderr`.
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`.
On iOS, stderr surfaces in `idevicesyslog` / Xcode.
So the iroh P2P transport's `info!`/`warn!`/`debug!` lines (endpoint bound + node id, each connect attempt's target + outcome, "delta sync received N ops") are visible while debugging device↔device sync.
Init uses `set_global_default`, not `.try_init()`: `try_init` also claims the `log` slot, which `tauri-plugin-log` needs.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) for what the transport logs.

## iCloud layout (opt-in destination)

iCloud is one destination the chosen folder may live in, never the default; opting in puts the root at `<ubiquity-container>/Documents/`.
Layout + the undotted-path trap: [`docs/ios-platform.md`](../../docs/ios-platform.md#icloud-layout-opt-in-destination).

## Peer-file materialisation (the iCloud catch)

iCloud syncs file metadata eagerly and content lazily, so a freshly notified peer `ops-<actor>.jsonl` can exist at zero bytes.
`main.mm`'s `OutlOpsWatcher.onUpdate:` forces materialisation before notifying the frontend — skip it and you race the download daemon.

The two mandatory calls: [`docs/ios-platform.md`](../../docs/ios-platform.md#peer-file-materialisation-the-icloud-catch).

## Bundle / signing

`gen/apple/Outl.xcconfig` owns the app's identity (bundle id, App Group, URL scheme, capture notification, display name); Swift reads it back through `OutlKit`'s `OutlFlavor`.
**Never write an identity literal into a plist, an entitlements file or a Swift file.**
That is the ten-place lockstep this replaced.
`tauri.conf.json` → `identifier` is the second owner (the CLI does not read an xcconfig) and has to agree.
On a phone holding real notes run `scripts/ios-preview.sh <device>`, which installs beside the real app instead of over it.
Identifiers, team, entitlements, the derivation table and the preview flow: [`docs/ios-platform.md`](../../docs/ios-platform.md#bundle--signing).

## Running

Simulator, physical device and release-archive commands: [`docs/development.md`](../../docs/development.md#mobile-ios-simulator).

## Versioning + TestFlight release

**Single source of truth: `Cargo.toml` workspace `version`.**
To bump the app version, edit `[workspace.package].version` at the repo root — the Rust crate, the Tauri config, `CFBundleShortVersionString` and `MARKETING_VERSION` all inherit from there.

**Never** put `"version": "x.y.z"` back in `tauri.conf.json`.
The iOS code path does NOT honor Tauri's `Cargo.toml` fallback (it uses `1.0.0`), so CI injects the workspace version via `cargo tauri ios build --config`.
A static field in the file would win over that override and the two drift on the next bump.

The field-resolution table, the `CFBundleVersion` (build number) scheme, and the three-workflow CI release flow are in [`docs/development.md`](../../docs/ci-and-release.md#ios-version-propagation-and-testflight).

## Deep links (`outl://`)

The scheme contract, the shared `outl_actions::parse_deep_link` parser, and this client's warm / cold wiring live in [`docs/deep-links.md`](../../docs/deep-links.md#mobile-wiring-outl-mobile).
Two things to keep in mind before touching it: scheme registration is the iOS `Info.plist` (`CFBundleURLTypes`), **not** `tauri.conf.json` (that key is desktop-only), and validation needs a device build — a host `cargo check` proves nothing here.

## Testing

The two layers (Rust commands + storage, frontend pure logic), their tooling and what each covers: [`docs/development.md`](../../docs/development.md#per-client-test-suites).

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-mobile -- -D warnings`
3. `cargo test -p outl-mobile`
4. `bun run test` (Vitest, frontend)
5. Build pass: `cargo tauri ios build`

## Trust surfaces

- **Page history**: `components/HistorySheet.tsx`, opened from the page properties sheet (long-press the title, then "Page history"). Same `restorePlan` and `DiffText` as the desktop `TimelinePanel`; restores are new ops.
- **Devices sheet** shows per-device op counts (`device_activity`) and the Markdown export (`export_markdown`), which lands next to the workspace, so in iCloud Drive it shows up in Files.
- **Saved mark**: `lib/saved.ts`; `Journal`'s commit marks a block only after `edit_block` resolves, `BlockRow` renders the dot.

## Polish: motion, focus mode, first-run tips

- `BlockRow` wraps the row (not its children) in `outl-row` so focus mode dims rows without compounding, and slides children in with `outl-unfold` only on an unfold. `SyncDot` pulses once when a pass ends.
- Focus mode is the capsule button between the sync dot and Settings, same as desktop. Header and `BottomBar` carry `outl-focus-hide` and fade to 0.18, never to 0, so the button stays tappable.
- `PageSections` gets `blank` from `Journal` and shows `<FirstRunTips client="mobile" />`; `JournalEmpty` shows it on the start and empty-page states. The tip names the indent keys, never a swipe: a swipe deletes a row here.
- Refs, tags and links in block text and backlinks use `MarkdownInline` `variant="inline"`, the same underlined look as desktop, not pills. The 44pt touch target comes from `.outl-ref::after` in `src/styles.css`; keep it if you touch either. The TODO box in `BlockMarkers` is a square (`rounded-[4px]`) inside a 44pt button, matching desktop.

## The backend's log, and why it needed one

`src-tauri/src/lib.rs` installs a `tracing` subscriber writing to stderr, with a comment saying that on iOS stderr "surfaces in idevicesyslog / Xcode".
It does not.
A release build on the simulator emits **nothing** to `simctl launch --console`, `--console-pty` or `log stream`; the only lines that arrive come from dylibs using `NSLog`, which goes through `os_log`.
Rust's `io::stderr()` is not wired to anything on either simulator or device.

So every `info!` / `warn!` the backend produced — the sync transport, the workspace opener, every boot repair, every projection refusal — was invisible on the one platform where the app is hardest to debug and where there is no `outl doctor` to fall back on.
A blank-screen report could only be narrowed by elimination, which is exactly as slow as it sounds.

`backend_log` is the sink: `$HOME/Library/Logs/outl-backend.log` inside the app sandbox, **truncated on every launch**, still mirrored to stderr so a desktop-style run is unchanged.
Truncating rather than rotating is the right trade on a phone: what you want from a backend log there is "what did *this* launch do", and a file holding one launch cannot grow without bound on a device nobody prunes.

`tauri_plugin_log` owns the `log` crate's slot and writes the **frontend's** lines to its own rotating file; our `tracing::` events deliberately bypass it (see `lib.rs` on why the two logger slots are kept apart), which is why they need a sink of their own.

Read it with:

```sh
cat "$(xcrun simctl get_app_container <udid> app.outl.mobile-app data)/Library/Logs/outl-backend.log"
```

`outl_core::boot_timing` is what makes that worth reading on a launch: one line per workspace open, naming each phase plus the delta size and resident block count that explain them.

## The workspace open and the first sync must not run at the same time

`wire_iroh_transport` **builds** the transport during `setup` (identity + peer store, no socket) and hands it to `AppState`, but **starts** it only after the background opener signals through a `std::sync::mpsc` channel.

Starting the transport binds the endpoint and immediately dials every known peer for an initial sync.
On a device meeting a large history for the first time that dial pulls the op log while `spawn_workspace_opener` is replaying it, and both peak in the hundreds of MB.
On a 723k-op workspace that killed the Android app with `Scudo ERROR: internal map failure` and a `SIGABRT` on a `tokio-rt-worker` — and the tell is that the launch had **no `boot phases:` line at all**, because the open never finished.

It is a channel and not the `workspace-ready` event on purpose: a channel cannot be missed.
The signal is buffered if the transport is not waiting on it yet (a small workspace opens faster than `setup` reaches the wiring), and a panicking opener drops the `Sender`, which wakes the waiter just the same rather than leaving the device with no sync at all.

A transport that exists but has not started is an already-supported state — `sync_now()` is a no-op and `peer_health()` is empty until `start()` runs — so the only cost is inbound dials during the open, which peers retry.
