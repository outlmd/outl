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

## Safe areas: the native shell measures, the page applies

`env(safe-area-inset-*)` is not trustworthy in either webview here: both shells are edge-to-edge (full-screen `WKWebView`; `enableEdgeToEdge()` in `MainActivity`), and with the inset reading 0 the header's `max(env(...), 12px)` fell to its floor and drew under the status bar.
So there is one source: `OutlSafeArea.swift` (iOS, a probe view's `safeAreaInsetsDidChange`) and `OutlSafeArea.kt` (Android, the `WindowInsets` listener, system bars plus display cutout, IME excluded) push CSS-pixel values into the page, and `src/lib/safe-area.ts` writes them to `--safe-area-inset-*` on `:root`, installed once from `index.tsx`.
`styles.css` declares those properties from `env(..., 0px)` as the fallback.
**A component reads `var(--safe-area-inset-*)`, never `env()`**; `safe-area.css.test.ts` fails on a stray `env(safe-area-inset` and on a fallback without a length.
Load order is covered both ways: native pushes on change and when the webview appears, the page pulls (iOS `outlSafeArea` message handler, Android `window.OutlSafeArea.get()`).
Only a device can show the numbers, so check a notched iPhone in both orientations and an Android phone with gesture and three-button navigation before calling it done.

**A correct inset is half of it: the chrome also has to stay where the inset put it.**
The WKWebView scrolls the *document* on its own, and the journal header is ordinary content above the outline, so a document scroll takes it under the status bar and the header reads as having lost its inset when it has not.
That scroll is not CSS overflow (measured: `scrollTop` 384 with `scrollHeight === clientHeight === 874`), so `overflow: hidden` does not stop it, and neither does `position: fixed` on `#root`, because iOS moves fixed boxes with the visual viewport.
`src/lib/document-scroll-lock.ts` is the owner of the rule: with nothing focused for text entry, the document sits at the origin; while a text field holds the scroll it is left alone, because that is what keeps the caret above the keyboard.
`styles.css`'s `html, body { overflow: hidden }` is the other half, against a box that grows instead.
**Before blaming the inset for a header in the wrong place, read `document.scrollingElement.scrollTop`.**

## Per-feature behaviour lives in `docs/mobile-ux.md`

Nine sections moved out: how a ref that does not exist yet is opened, the page switcher's long-press delete, external links, blockquote chrome, zoom / focus, paste from external apps, the keyboard accessory bar, code execution, and template insertion.

They are reference — you look one up when you touch that feature — and this file is loaded into context on every session in this crate, where the budget belongs to the rules you must not break.
[`docs/mobile-ux.md`](../../docs/mobile-ux.md) is their owner now.

Two more parts of this client are reference rather than rule, and they live under `docs/` for the same reason:

| Page | What it covers |
|---|---|
| [`docs/mobile-components.md`](../../docs/mobile-components.md) | What came out of `Journal.tsx` and `BlockRow.tsx`, what each sibling owns, and where new code of each shape goes |
| [`docs/mobile-internals.md`](../../docs/mobile-internals.md) | How each surface is wired: onboarding, page switcher, properties, settings, reminders, plugins, peers, the sync dot, theming, the log, polish |

Read the matching one before editing those files.

## Cross-runtime contracts (now in `@outl/shared`)

TS pieces mirroring a Rust canonical source (`looksLikeOutline`, `<MarkdownInline />`, `detectRefContext`, `autoPairBracket`, `utf16OffsetToCharOffset`, …) used to live as copies under `lib/`.
Extracted to `crates/outl-frontend-shared/` so mobile and desktop share one file.
Contract-by-contract table (moved out to keep this file under the per-crate size ceiling): [`docs/primitives-markdown.md` § 9](../../docs/primitives-markdown.md#9-cross-runtime-ts-contracts-outlshared).
**A new contract goes in `@outl/shared` from day one**, never under a client's own `src/lib/` first.

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

## The workspace open and the first sync must not run at the same time

`wire_iroh_transport` **builds** the transport during `setup` (identity + peer store, no socket) and hands it to `AppState`, but **starts** it only after the background opener signals through a `std::sync::mpsc` channel.

Starting the transport binds the endpoint and immediately dials every known peer for an initial sync.
On a device meeting a large history for the first time that dial pulls the op log while `spawn_workspace_opener` is replaying it, and both peak in the hundreds of MB.
On a 723k-op workspace that killed the Android app with `Scudo ERROR: internal map failure` and a `SIGABRT` on a `tokio-rt-worker` — and the tell is that the launch had **no `boot phases:` line at all**, because the open never finished.

It is a channel and not the `workspace-ready` event on purpose: a channel cannot be missed.
The signal is buffered if the transport is not waiting on it yet (a small workspace opens faster than `setup` reaches the wiring), and a panicking opener drops the `Sender`, which wakes the waiter just the same rather than leaving the device with no sync at all.

A transport that exists but has not started is an already-supported state — `sync_now()` is a no-op and `peer_health()` is empty until `start()` runs — so the only cost is inbound dials during the open, which peers retry.
