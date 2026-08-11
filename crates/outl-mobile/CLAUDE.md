# CLAUDE.md — outl-mobile

Tauri 2 mobile client (iOS first, Android later).
Solid.js + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation is delegated to `outl-actions`.

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
   │   ├── android_jni.rs          (Android-only: primes rustls-platform-verifier + ndk_context before iroh's first QUIC connect)
   │   ├── plugin_service.rs       (PluginService + dedicated plugin thread — Boa Context is !Send, so it can't live in AppState)
   │   └── commands/               (Tauri command surface — split mirrors outl-desktop)
   │       ├── mod.rs
   │       ├── workspace.rs        (workspace_stats, reload_workspace)
   │       ├── page.rs             (list_all_pages / search_pages / search_persons / outl_emoji_search / open_* / *_day / resolve_ref / legacy compat shims)
   │       ├── block.rs            (create / edit / toggle_todo / toggle_quote / delete / indent / outdent / move_* / set_collapsed / paste_markdown_at / copy_markdown)
   │       ├── peers.rs            (outl_peer_list / outl_peer_remove — read/edit <workspace>/.outl/peers.json, no workspace lock)
   │       ├── plugin.rs           (plugin_list / plugin_run / plugin_sync_hooks — thin shims over PluginService)
   │       └── exec.rs             (run_code_block — thin shim over outl_actions::exec::run_code_block)
   ├── gen/apple/.../main.mm       (NSMetadataQuery + NSFileCoordinator iCloud watcher)
   ├── gen/apple/.../OutlBackgroundRefresh.swift  (BGTaskScheduler windows + the beginBackgroundTask flush)
   ├── gen/android/.../MainActivity.kt            (NativeSetup.install + OutlBackgroundSync.install, before Tauri boots)
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
there is no `icloud_storage.rs` because the only iCloud-specific work is resolving the ubiquity container path (via `icloud_path.rs`) and forcing peer-file materialisation before reads (via `OutlOpsWatcher.swift`).
The storage trait stays generic; the transport gets handled outside it.

## Storage is a chosen folder, not forced iCloud (Fase 2)

**The workspace root is a folder the user picks.**
It may live anywhere — the app's local data dir (the default), the Files app, or inside an iCloud container — and **iroh P2P is the primary sync**.
iCloud is now just *a place the folder might be*, never a hard dependency.
A fresh install works with **zero iCloud**.

Boot resolution (`workspace_open::resolve_storage_root`):

1. The persisted `WorkspaceCfg.last` path (from `outl-config`), when present and usable — survives restarts.
2. Else the app-local default `<app-data-dir>/outl/` — synced by iroh, no iCloud.

The old behaviour (force `<ubiquity-container>/Documents/`, fall back to local only if iCloud was unavailable) is gone.
iCloud is reachable on demand via `workspace_open::icloud_workspace_root()` (used by `workspace_picker::pick_in_icloud`).

**Folder selection.**
`workspace_picker.rs` owns the choice.

- `set_workspace(path)` validates, creates the dir, persists `WorkspaceCfg.last`, and emits `workspace-reopen-required`.
  The reopen is **boot-read** today (next launch picks up `last`); a runtime swap would need `AppState.storage_root` to become an `Arc<Mutex<Option<PathBuf>>>` plus an iroh rebind, deliberately deferred.
  Frontend wrapper: `setWorkspace(path) → Promise<void>` in `src/lib/api.ts`.
  No caller wires it yet — the arbitrary-folder native picker (`UIDocumentPickerViewController` + security-scoped bookmark) is deferred, so the local default is the only root a fresh install opens.

> **Registration note:** both commands are registered in `lib.rs`'s `invoke_handler!` list (`workspace_picker::set_workspace`, `workspace_picker::pick_in_icloud`).

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

## Background sync (iOS + Android)

Neither OS lets a backgrounded app keep syncing: iOS suspends the process and its sockets, Android freezes the cached process (cgroup freezer, API 30+, ~10s after caching on API 34+).
Same outcome — an iroh delta sync in flight is torn down and the peer logs `peer did not confirm durable ingest (closed: timed out)`.
So each platform gets a scheduler that **actively** drives a forced pass inside whatever window the OS grants.

**`bg_sync.rs` is the one owner of that pass, for both.**
`Registration` / `register` / `drive_sync` / `wait_until` / `registered_peer_count` are platform-agnostic and unconditional.
Only the **exported symbols** are `cfg`-gated — `target_os = "ios"` for the C ABI `@_silgen_name` binds against, `target_os = "android"` for the JNI symbols `NativeSync.kt` declares.
That split is deliberate and load-bearing: it is what keeps the shared bodies covered by the host test suite, where neither platform's exports compile.
Adding a fourth operation means adding it to the core plus *both* export blocks — never to one platform's block alone.

Two rules that outlive any refactor here:

1. **Wait on your own pass, never on "the counter moved".**
   `drive_sync` calls `IrohSyncTransport::sync_now_seq()` and waits for `completed_sync_passes() >= seq && peers_in_flight() == 0`.
   The naive version shipped and was wrong.
   The completed-pass counter is global and `Journal.tsx` fires `syncNow()` on a 3s foreground timer, so a background flush watched the *foreground* pass complete ~250ms later and released the OS window with its own request still queued.
   A `seq` of `0` means the runtime is down — return immediately, do not burn the cap waiting for a request that was never enqueued.
2. **Never panic or throw across the boundary.**
   No `unwrap`/`expect` in this module; the JNI wrappers additionally go through `with_env` (`catch_unwind`) + `LogErrorAndDefault`, so a failure degrades to `JNI_FALSE` / `0` instead of failing the Kotlin job or aborting the process.

Per-platform wiring, and what each one does **not** guarantee:
[`docs/ios-platform.md`](../../docs/ios-platform.md#background-sync-ios) · [`docs/android-platform.md`](../../docs/android-platform.md#background-sync-android).

The Android side needs `androidx.work:work-runtime` **≤ 2.10.5** until the Kotlin Gradle plugin moves off 1.9.25.
2.11.x pulls `kotlin-stdlib:2.1.20`, whose metadata the 1.9 compiler cannot read — and that breaks every file in the module, including Tauri's generated ones.

Neither path can be validated on a host or a simulator.
iOS's simulator has no `BGTaskScheduler` daemon (`submit` always fails); Android's schedules need `adb shell cmd jobscheduler run`, and the real acceptance test is two paired devices with no timed-out-ingest row after locking the phone.

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
Detection is `splitQuote` + `stripQuoteFromTokens` (`@outl/shared/markdown`, mirror of `outl_actions::quote::split_quote`) so the `> ` isn't rendered twice; it composes with the TODO/DONE checkbox.
Toggling: `toggleQuote(id)` → `toggle_quote` → `outl_actions::block::toggle_quote` (no TS string surgery).
Convention (three-surface parity): [`docs/clients.md` → Blockquote convention](../../docs/clients.md#blockquote-convention).

## "This page isn't syncing" banner

`<PageAheadOfLogBanner client="mobile" />` (from `@outl/shared/warnings`) renders above the outline when `PageView.md_ahead_of_log` comes back set.
That means the page's `.md` holds lines the op log never recorded, so outl refuses to overwrite it and the page has stopped converging with the user's other devices.
The copy is owned by `@outl/shared/warnings::aheadOfLogNotice`, never written here; `client="mobile"` is what makes it say "open this workspace on your computer" instead of naming a terminal command, because **there is no `outl` binary on iOS**.
The flag is held in `Journal.tsx`'s own `aheadOfLog` signal, **not** read off `view()`: only the open commands carry it, so an edit commit's reply would otherwise clear the banner on the user's first edit — the very action it warns against.
It is cleared by a reply carrying `md_ahead_of_log_checked` with no notice (the next open / refresh once the page is healthy again), so the banner can't outlive the condition — same rule as desktop.
Convention: [`docs/clients.md` → Surfacing a page that stopped syncing](../../docs/clients.md#surfacing-a-page-that-stopped-syncing).

## Zoom / focus on a block

Tap a block's plain bullet dot to zoom in — it becomes the outline root (Roam/Workflowy focus); `← Back` + breadcrumb zoom out.
Local view state (`focusBlockId` in `Journal.tsx`), never a Tauri round-trip; the shared `focusSubtree` (`@outl/shared/outline`) does the subtree + breadcrumb.
Mobile owns only the touch chrome: the bullet tap moves mark-as-TODO to the long-press menu; checkbox + `<CollapseTriangle>` untouched.
Convention: [`docs/clients.md` → Zoom / focus on a block](../../docs/clients.md#zoom--focus-on-a-block-roamworkflowy).

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
Convention (shared `dispatchToolbarAction`, the two invariants): [`docs/clients.md` → Keyboard accessory bar](../../docs/clients.md#keyboard-accessory-bar-mobile).

## Code execution (`run_code_block`)

Long-press a `` ```lang …``` `` block → "Run `<lang>`" fires `runCodeBlock`.
Mobile's `src-tauri/src/exec.rs` is a **thin adapter** over `outl_actions::exec::run_code_block` (shared with desktop), wrapping the outcome with a refreshed `PageView`.
The action only shows when `detectFence` matches; the backend re-validates in `run_block_at_index`, so a false-positive is a toast, not damage.
Runtimes on iOS: **Lisp, JS, Python, Lua** — `lang-rust` is off in `Cargo.toml`.
Flow + runtime-catalog rationale: [`docs/clients.md` → Running code blocks](../../docs/clients.md#running-code-blocks).

## Insert template (structural templates)

The block long-press menu's "Insert template" action opens `TemplateSheet` (bottom sheet listing `listTemplates()`); picking one calls `instantiateTemplateAt(name, blockId)` and applies the returned `PageView`.
Wire commands are the shared `list_templates_cmd` / `instantiate_template_at` bodies — no mobile logic; contract in [`docs/clients.md` → Structural templates](../../docs/clients.md#structural-templates).

## Reminders (`remind::`)

The header bell opens `<RemindersSheet />`, a block long-press authors a rule via `set_block_remind`, and `Journal.tsx` polls `deliver_due_reminders` every 30s.
Sheet wiring, why authoring is a prompt instead of a native time picker, and why both device-local settings live in the sheet: [`docs/reminders.md`](../../docs/reminders.md#mobile-outl-mobile).
The one rule that matters here: **the schedule is never computed in TS** — `groupReminders` / `formatNextFire` only format what `outl_actions::reminders` decided.

## Plugins

JS plugins (`outl_plugins::PluginHost`) run on mobile; the design is the desktop's.
Read [`outl-desktop/CLAUDE.md` → Plugins](../outl-desktop/CLAUDE.md#plugins) for the shared rationale (`!Send` Boa host on a dedicated thread, `PluginService` in `AppState`, re-projection via `apply_all_pages_md`).
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
The registry + `(block id, body)` cache glue is shared with the desktop in `@outl/shared/plugins/transformer-registry` (see [`outl-desktop/CLAUDE.md` → Plugins](../outl-desktop/CLAUDE.md#plugins)).
Mobile just wires it: `Journal.tsx`'s `onMount` calls `loadTransformers()`, and `BlockRow`'s fence branch renders `<PluginFence />` on a `transformerFor(lang)` match (else plain `<HighlightedCode />`).
`rich` output lands in an inline sandboxed `<iframe>` — `allow-scripts`, never `allow-same-origin`, same posture as `<PluginViewOverlay />`.

## Peer / device management (`outl_peer_list` / `outl_peer_remove`)

`commands/peers.rs` exposes two Tauri commands over the iroh peers file (`<workspace>/.outl/peers.json`, via `outl_sync_iroh::PeersStore`):

- `outl_peer_list() -> Vec<PeerDto>` — lists paired devices (`node_id`, `alias`, `added_at`).
- `outl_peer_remove(id: String) -> bool` — removes peers whose `node_id` starts with the prefix; `true` if any matched.

The peer list is per-**graph** (resolved from `AppState::storage_root` via `outl_sync_iroh::workspace_peers_path`), NOT next to the device identity.
The device `identity.key` stays per-**install** in the Tauri app local data dir ([`iroh_sync::iroh_dir`]) — one node id per install.
Each command runs `migrate_global_peers_if_absent` first, so a legacy global
`~/.outl/peers.json` is copied into the workspace once on first open.
These are the **only** commands that touch `peers.json` directly instead of
the workspace lock — the list is graph-scoped sync-transport state, so they
read `storage_root` without going through `outl-actions`.

`commands/peers.rs` also exposes `outl_sync_now()` (reads `state.iroh`, calls the transport's `sync_now()`) — the force-sync trigger behind the refresh button.

## Sync dot + refresh (iroh-driven)

The header `<SyncDot>` and the refresh button / `PullToRefresh` reflect and drive the **iroh P2P transport** (outl's default sync), not the iCloud-era `navigator.onLine` signal they started on.

- **Dot state.**
  The PRIMARY input is iroh peer health, polled via `peerStatus()` → the shared `peersOnline()` helper (`@outl/shared/peers`) into a `peersUp` signal.
  The poll runs on mount, every 5s, and after each `peer-ops-changed` (the native ops bridge) plus after a force-sync.
  Derivation: a force-sync in flight → **syncing** (spinner); else `online() && peersUp()` → **synced** (green); else **offline** (orange).
  `navigator.onLine` stays only as a secondary floor (truly no radio → orange regardless).
  Zero paired peers reads as offline — there's nothing to sync with.
- **Refresh.**
  `handleRefresh` (the button **and** `PullToRefresh`) calls `syncNow()` (force a P2P pull — dial every peer now instead of waiting for the 8s catch-up tick) THEN `reloadWorkspace()` (re-render with whatever landed).
  Both calls are wrapped in `withError` (toast on failure, never wedge the local reload), and the `syncing` spinner brackets the whole pass.
- **Auto-sync (no button).**
  `Journal.tsx` shares the refresh core as `pullAndReload()`, fired on `onMount`, on `visibilitychange` → visible (iOS froze JS in the background), and on the 5s poll tick.
  The mobile side initiating the dial is NAT-friendly — waiting for the desktop to reach an iPhone behind carrier NAT is not — so this is what makes a desktop edit show up without the user touching refresh.
  The `workspace-ready` reload skips while a block is being edited (guarded by `editingId()`) so it never resets the textarea mid-edit.
  It also routes through `pullAndReload` (not a raw `openJournalFor` + `applyView`) so it inherits its guards.
  **Reload-ordering guard:** each `pullAndReload` captures a monotonic `reloadGen` at entry and applies only when still the latest, so a slow reload can't flip the page back to an older state.
  (The split-content flicker's real cause was a backend duplicate-slug-root bug, fixed in `outl-actions::merge_duplicate_slug_roots`; this is the frontend belt-and-suspenders.)

`syncNow()` and `peersOnline()` both live in `@outl/shared` so mobile and desktop derive the dot + drive the refresh identically.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

## Cross-runtime contracts (now in `@outl/shared`)

The four TS pieces that mirror Rust canonical sources used to live as copies under `lib/`.
They were extracted to **`crates/outl-frontend-shared/`** so mobile and desktop import the same file — drift between two TS implementations is geometrically impossible.

| Contract | Path | Mirrors (Rust) |
|---|---|---|
| `looksLikeOutline` | `@outl/shared/paste` | `outl_actions::paste::looks_like_outline` |
| `<MarkdownInline />` (renderer of `InlineToken[]`) | `@outl/shared/markdown` | `outl_md::tokenize_owned` (backend produces the tokens; the renderer is a discriminant-to-JSX switch) |
| `detectRefContext` (+ `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion`) | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::detect_trigger` (the `[[` and `((` triggers; TUI also covers `#` and `/`) |
| `autoPairBracket` (auto-pair `(`/`[`/`{` + step over auto-inserted closers; wired via `onBeforeInput` since iOS soft keyboards skip per-char `keydown`) | `@outl/shared/autocomplete` | `outl_tui::input::insert` (`insert_pair`) + `EditBuffer::delete_pair_back` |
| `utf16OffsetToCharOffset` | `@outl/shared/paste` | runtime gap, no Rust mirror — `selectionStart` is UTF-16, the backend expects codepoints, or the splice shifts per supplementary-plane char |

**Adding a new cross-runtime contract = add it in `@outl/shared` from day one.**
Never add it under `outl-mobile/src/lib/` first — the next time desktop catches up to the feature, it has to consume from the same file.

## Logging (device console)

`run()` in `src-tauri/src/lib.rs` installs a `tracing_subscriber` fmt subscriber writing to **stderr** as its very first step (before rustls / Tauri setup).
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`.
On iOS, stderr surfaces in `idevicesyslog` / Xcode.
So the iroh P2P transport's `info!`/`warn!`/`debug!` lines (endpoint bound + node id, each connect attempt's target + outcome, "delta sync received N ops") are visible while debugging device↔device sync.
Init uses `.try_init()` so a double-init can't panic.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) for what the transport logs.

## iCloud layout (opt-in destination)

iCloud is one destination the chosen folder may live in, never the default; opting in puts the root at `<ubiquity-container>/Documents/`.
Layout + the undotted-path trap: [`docs/ios-platform.md`](../../docs/ios-platform.md#icloud-layout-opt-in-destination).

## Peer-file materialisation (the iCloud catch)

iCloud syncs file metadata aggressively and file content lazily, so a freshly notified peer `ops-<actor>.jsonl` can read as an empty placeholder — truncated op log, wrong merge, broken `.md` written back.
`main.mm`'s `OutlOpsWatcher.onUpdate:` forces materialisation before notifying the frontend; the two mandatory steps are in [`docs/ios-platform.md`](../../docs/ios-platform.md#peer-file-materialisation-the-icloud-catch).
Skip either and you race the iCloud download daemon.

## Bundle / signing

Bundle id + iCloud container are **global** in the Apple Developer ecosystem, so changing either means updating six files in lockstep.
Identifiers, team, entitlements and that checklist: [`docs/ios-platform.md`](../../docs/ios-platform.md#bundle--signing).

## Running

Simulator, physical device and release-archive commands: [`docs/development.md`](../../docs/development.md#mobile-ios-simulator).

## Versioning + TestFlight release

**Single source of truth: `Cargo.toml` workspace `version`.**
To bump the app version, edit `[workspace.package].version` at the repo root — the Rust crate, the Tauri config, `CFBundleShortVersionString` and `MARKETING_VERSION` all inherit from there.

**Never** put `"version": "x.y.z"` back in `tauri.conf.json`.
The iOS code path does NOT honor Tauri's `Cargo.toml` fallback (it uses `1.0.0`), so CI injects the workspace version via `cargo tauri ios build --config`.
A static field in the file would win over that override and the two drift on the next bump.

The field-resolution table, the `CFBundleVersion` (build number) scheme, and the three-workflow CI release flow are in [`docs/development.md`](../../docs/development.md#ios-version-propagation-and-testflight).

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
