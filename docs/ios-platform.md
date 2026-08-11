# iOS platform integration (mobile client)

What the iOS shell of `outl-mobile` needs from outside Rust.
Three things: the bundle identifiers Apple treats as global, the background-sync wiring across Info.plist / Swift / FFI, and the iCloud catches that only exist when the workspace folder lives in a ubiquity container.
Every item here can only be validated with a **device or simulator build** — a host `cargo check` proves nothing about it.

The crate contract (what stays thin, what delegates to `outl-actions`) lives in [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md).
The user-facing background-sync behaviour is [sync.md → Background sync on iOS](sync.md#background-sync-on-ios); the build + release commands are [development.md](development.md#mobile-ios-simulator).

---

## Bundle / signing

- Bundle id: `app.outl.mobile-app`
- Team: `CPEEKT3E77` (paid Apple Developer Program)
- iCloud container: `iCloud.app.outl.mobile-app`
- Display name (Files.app / iCloud Drive): `outl`
- Category: `public.app-category.productivity`
- Entitlements: `com.apple.developer.icloud-services` + `icloud-container-identifiers` + `ubiquity-container-identifiers`

Bundle ID + iCloud container are **global** in the Apple Developer ecosystem.
If you change either, also update:

1. `tauri.conf.json` → `identifier`
2. `src-tauri/src/lib.rs` → `ICLOUD_CONTAINER_ID`
3. `gen/apple/outl-mobile.xcodeproj/project.pbxproj` → `PRODUCT_BUNDLE_IDENTIFIER`
4. `gen/apple/outl-mobile_iOS/outl-mobile_iOS.entitlements`
5. `gen/apple/outl-mobile_iOS/Info.plist` → `NSUbiquitousContainers` key
6. `gen/apple/project.yml` → `bundleIdPrefix` and `PRODUCT_BUNDLE_IDENTIFIER`

---

## Background sync (iOS)

iOS suspends the app's sockets shortly after it backgrounds, so there is **no continuous background P2P**.
Three mechanisms cover three different moments, and the first one is not a `BGTaskScheduler` window at all:

**0. The handover — finishing the pass that is already running.**
Locking the screen suspends the process within seconds, tearing down whatever delta sync is mid-flight.
That is *safe*: the responder confirms durable ingest by closing with code 0, so an interrupted push is re-sent on the next tick.
It is also **visible** — the peer logs `peer did not confirm durable ingest (closed: timed out)`, and the desktop Sync panel shows a red row every time the user pockets their phone.
Neither `BGTaskScheduler` window helps here — both are requests for a window *later*, at the scheduler's discretion.
`OutlBackgroundRefresh.flushOnBackground()` closes it with a `beginBackgroundTask` assertion on `didEnterBackground`.
That buys ~30s of runtime in which the process stays resident, so the forced pass it drives **and** any inbound sync a peer is mid-way through both complete.
It needs no `UIBackgroundModes` entry — an assertion is not a mode.
The rules that matter, each of which a first version got wrong:

- **End it exactly once.**
  The expiration handler is a hard deadline — iOS *terminates* an app that overruns — and `endBackgroundTask` must never run twice on one identifier.
  `endFlush(matching:)` takes the identifier the caller owns, because a foreground bounce can end flush A while its worker still runs, and that worker would otherwise end **B's** assertion.
- **Release on your own pass, not on any pass.**
  The FFI waits for the sequence number `sync_now_seq()` returned *and* for `peers_in_flight()` to reach zero.
  Waiting on "the completed-pass counter moved" reads the foreground timer's pass (mobile fires one every 3s) as your own and releases the window ~250ms in.
- **Size the window from `backgroundTimeRemaining`,** not a constant.
  One unreachable peer costs 5s direct + 10s relay, so a fixed 20s cap overran a real budget with two peers — guaranteeing the tear-down the assertion exists to prevent.
  Clamped to 3–20s with 5s held back so the FFI returns before the expiration handler fires.
- **Take it synchronously.**
  The `didEnterBackground` observer registers with `queue: nil`; an `OperationQueue` hop is exactly the window in which iOS begins suspending.
- **Skip it** with zero paired peers (checked on the worker, not on the main thread — it reads `peers.json` off disk) or a denied (`.invalid`) assertion, and never re-enter while one is in flight.

**Then the later windows** — the two opportunistic `BGTaskScheduler` tasks, numbered 1 and 2 below.
**Both** sync, wired across three pieces:

1. **Info.plist** declares `UIBackgroundModes` (`fetch` + `processing`) and `BGTaskSchedulerPermittedIdentifiers` (`app.outl.mobile-app.refresh`, `app.outl.mobile-app.sync`).
   Without these the toggle never shows in Settings and `BGTaskScheduler.register`/`submit` fail silently.
2. **`OutlBackgroundRefresh.swift`** registers both tasks (`+load` → `install`) through one shared `handleTask` helper (reschedule first, FFI on a background queue, complete exactly once — the work and the OS expiration handler race).
   The `refresh` (`BGAppRefreshTask`, ~30s windows) drives the short FFI; the `sync` (`BGProcessingTask`, `requiresNetworkConnectivity = true`) drives the long one.
   **Scheduling is gated on having paired peers** (`outl_ios_peer_count() > 0`) so an unpaired device never boots the stack for nothing.
   A `didEnterBackgroundNotification` observer re-submits on every backgrounding, which also arms the gate right after the first pairing.
3. **`bg_sync.rs`** owns the two FFIs (C ABI, `@_silgen_name` on the Swift side).
   They are `outl_ios_background_sync_capped(seconds)` and `outl_ios_peer_count()` (reads `<root>/.outl/peers.json` fresh from disk, so post-boot pairings count).
   One capped symbol serves all three windows — each caller passes its own ceiling (12s refresh, 20s processing, `backgroundTimeRemaining` for the flush) and Rust clamps it, so an expired budget can't hang the thread.
   `wire_iroh_transport` registers a `Clone` of the live `IrohSyncTransport` **plus the workspace root** into a re-settable global.
   The sync FFIs fire `sync_now()` (a forced delta-sync against every peer, mobile side initiating, which is NAT-friendly).
   They then poll `completed_sync_passes()` every 250ms, returning as soon as the pass lands — the cap is a fallback, not a fixed sleep.

The FFI + Swift handler can only be validated with a **device build**.
The simulator has no `BGTaskScheduler` daemon, so `submit` always fails there and is swallowed; the Rust side is `cargo check`-clean on its own.

---

## iCloud layout (opt-in destination)

When the user opts into iCloud, the root is `<ubiquity-container>/Documents/` (`workspace_open::icloud_workspace_root()`) — **one option**, not the default.
The container is already the `outl` namespace, so no extra `outl/` nesting; the TUI uses `--path "<container>/Documents"`.
Layout is the standard `journals/` + `pages/` (`.md` + `.outl` sidecar) + `ops/` (one `ops-<actor>.jsonl` per device).
**iCloud trap:** every path must be undotted — iCloud Documents skips `.`-prefixed paths across devices, so `ops/` (not `.ops/`) and `pages/<slug>.outl`, else the file never leaves its origin.

---

## Peer-file materialisation (the iCloud catch)

iCloud syncs file metadata aggressively and file content lazily.
When `NSMetadataQuery` fires on a peer's `ops-<actor>.jsonl`, the file's bytes may not be on disk yet — a `std::fs::open` returns an empty placeholder.
The Rust side sees a truncated op log; the merge is wrong; the projection writes a broken `.md` back.

`main.mm`'s `OutlOpsWatcher.onUpdate:` works around this in two steps:

```objc
[fm startDownloadingUbiquitousItemAtURL:url error:&startErr];
NSFileCoordinator *coord = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
[coord coordinateReadingItemAtURL:url
                          options:NSFileCoordinatorReadingForUploading
                            error:&coordErr
                       byAccessor:^(NSURL *u) { (void)u; }];
```

`startDownloadingUbiquitousItemAtURL` requests materialisation; `NSFileCoordinator` blocks until the file is fully on disk.
Only after that does the watcher fire `window.__outlOpsChanged()` so the frontend can call `reload_workspace`.
Skip either step and you race the iCloud download daemon.
