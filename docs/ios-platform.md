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
  The FFI waits for the sequence number `sync_now_seq()` returned *and* for `inbound_serves()` to reach zero.
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

## LAN peer discovery (Bonjour, and why not multicast)

Same-LAN peer discovery ([issue #149](https://github.com/outlmd/outl/issues/149)) reaches every client through `outl-sync-iroh`'s `bind::attach_mdns`, but iOS is the one platform that cannot take the path behind it.

`iroh-mdns-address-lookup` speaks mDNS over a plain BSD socket — `bind` on `0.0.0.0:5353`, then `IP_ADD_MEMBERSHIP` on `224.0.0.251`.
Since iOS 14 that group join needs `com.apple.developer.networking.multicast`, a restricted entitlement Apple grants by request and **commonly declines**, with the suggestion to use Bonjour.

So iOS does not join a multicast group at all.
`OutlBonjour.swift` asks `mDNSResponder`, the system's own mDNS daemon, to advertise and browse on the app's behalf.
Apple's [Local Network Privacy FAQ](https://developer.apple.com/forums/thread/663875) names exactly two Bonjour operations that still require the entitlement — working with **arbitrary** service types, and browsing for advertised service types (`_services._dns-sd._udp.local.`) — and outl does neither.

> **Never add `com.apple.developer.networking.multicast` to the entitlements.**
> It is not needed, and an entitlement the provisioning profile does not carry **fails code signing**, breaking the TestFlight pipeline.

Two `Info.plist` keys are what this does need, and both are already in place:

| Key | Why |
|---|---|
| `NSBonjourServices` = `_irohv1._udp` | The fixed service type. Declaring it is what keeps this out of the "arbitrary service type" case that would need the entitlement |
| `NSLocalNetworkUsageDescription` | The reason string on the one-time local-network prompt |

### Why `NetService`, not `NWListener` / `NWBrowser`

`Network.framework` is the modern API and does not fit, in both directions:

- **Publishing.** `NWListener` advertises a port *it* opened. The port that has to be advertised is the one iroh's QUIC socket already holds, and no `Network.framework` type will advertise a socket it did not create. `NetService(domain:type:name:port:)` publishes a record for an arbitrary port.
- **Browsing.** `NWBrowser` returns an opaque `NWEndpoint.service` by design — the framework wants you to connect *through* it rather than learn addresses. iroh needs the actual socket addresses to hand to QUIC, and `NetService.addresses` is what exposes them.

`NetService` is soft-deprecated, not removed, and deprecation has no bearing on App Review.

### The failure mode that has no error

If the user declines the local-network prompt, **iOS tells the app nothing**.
Browsing continues, resolves nobody, and looks exactly like a LAN with no peers on it.
Nothing in the app can distinguish the two, so there is no in-app warning to write — when an iPhone never finds a peer its laptop finds fine, the answer is **Settings › Privacy & Security › Local Network › outl**.

Sync still works meanwhile: the relay path is untouched.

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
