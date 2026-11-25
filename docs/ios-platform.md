# iOS platform integration (mobile client)

What the iOS shell of `outl-mobile` needs from outside Rust.
Three things: the bundle identifiers Apple treats as global, the background-sync wiring across Info.plist / Swift / FFI, and the iCloud catches that only exist when the workspace folder lives in a ubiquity container (no shipping build puts it there today, see [iCloud layout](#icloud-layout-opt-in-destination)).
Every item here can only be validated with a **device or simulator build** — a host `cargo check` proves nothing about it.

The crate contract (what stays thin, what delegates to `outl-actions`) lives in [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md).
The user-facing guide to the app is [mobile.md](mobile.md), and the home-screen, Lock Screen, Control Center and share-sheet entry points a user can set up are [mobile-capture.md](mobile-capture.md).
The user-facing background-sync behaviour is [sync.md → Background sync on iOS](sync.md#background-sync-on-ios); the build + release commands are [development.md](development.md#mobile-ios-simulator).

The text App Review needs for a submission (App Review Information > Notes, plus the screen recording script) lives in [App Store review notes](app-store/review-notes.md).

---

## Bundle / signing

- Bundle id: `app.outl.mobile-app`
- Team: `CPEEKT3E77` (paid Apple Developer Program)
- iCloud container: `iCloud.app.outl.mobile-app`
- Display name (Home Screen, Files.app): `outl`
- Category: `public.app-category.productivity`
- Entitlements: `com.apple.security.application-groups` only, holding `group.app.outl.mobile-app`.
  **There are no iCloud entitlements and no `NSUbiquitousContainers` key.**
  Both went with the Rust-side iCloud teardown, and this page listed them for months after they were gone.
  A workspace folder inside an iCloud container still syncs, because that is the OS syncing a path the user picked, not the app holding a container.
- Export compliance: **not declared**, deliberately. See below.

### `gen/apple/Outl.xcconfig` is the one owner

Bundle ID is **global** in the Apple Developer ecosystem, and it used to be written out everywhere it was needed.
`PRODUCT_BUNDLE_IDENTIFIER` twice per target across three targets, two entitlements files, three `Info.plist` files, `project.yml`, and two Swift literals.
Ten spellings of one fact, each of which failed quietly when missed, and a checklist is what you get instead of a mechanism.

`Outl.xcconfig` declares it once, on both project-level build configurations, so the three targets inherit it:

| setting | production value | where it lands |
|---|---|---|
| `OUTL_BUNDLE_ID` | `app.outl.mobile-app` | `PRODUCT_BUNDLE_IDENTIFIER` on **the two extension targets**, suffixed `.widget` / `.share` |
| `OUTL_APP_GROUP` | `group.$(OUTL_BUNDLE_ID)` | both entitlements files, and `OutlAppGroup` in each `Info.plist` |
| `OUTL_URL_SCHEME` | `outl` | `CFBundleURLSchemes`, and `OutlURLScheme` in each `Info.plist` |
| `OUTL_CAPTURE_NOTIFICATION` | `app.outl.capture.arrived` | `OutlCaptureNotification` in each `Info.plist` |
| `OUTL_DISPLAY_NAME` | `outl` | `CFBundleDisplayName` on the app and both extensions |

`PRODUCT_NAME` is deliberately **not** in that table and stays `outl`: the Tauri CLI locates the built bundle by that name, so the Home Screen label is `CFBundleDisplayName` instead.

**Swift reads these out of `Info.plist`, through `OutlKit`'s `OutlFlavor`**, rather than repeating the literals.
That covers the three names an App Group does not scope, and which therefore reach across two installs on one phone.
The group itself.
The URL scheme, since iOS picks one app for `outl://` in whatever order it likes.
And the Darwin notification an extension posts after a capture, because `CFNotificationCenterGetDarwinNotifyCenter` is machine-global.

`OutlFlavor` falls back to the production values for a bundle with no such keys, because `swift test` runs outside an app bundle.
**That fallback is not the safety mechanism and must not be read as one.**
The entitlement is.
It is generated from the same `OUTL_APP_GROUP`, so a build whose plist somehow named another flavor's group is refused by the sandbox at the container lookup.
The failure is a build that cannot write its own captures, never one that writes into the wrong graph.

**Two owners remain and they have to agree**, because the Tauri CLI does not read an xcconfig:

1. `src-tauri/tauri.conf.json` → `identifier` owns the **app** target's bundle id, and what the CLI installs and launches.
2. `Outl.xcconfig` owns everything the CLI does not know about: the two extension targets, the App Group, the scheme, the notification, the display name.

The split is not a preference. **`cargo tauri ios dev` and `ios build` write `identifier` into the app target's build settings on every run**, so a variable there is overwritten on the next build and shows up as a diff in a committed file.
The app target therefore keeps the literal, and the extensions, which the CLI does not touch, derive from the xcconfig.

Changing the shipping bundle id means changing both.

### A preview build, installed beside the real app

`cargo tauri ios dev <device>` installs over the app from TestFlight: same bundle id, so it takes that container, that paired iroh identity and that graph.
Driving a dev build then writes synthetic edits into real notes, and the signed app is gone until the next TestFlight install.

```bash
cd crates/outl-mobile
scripts/ios-preview.sh "samba-phone"           # release, the default
scripts/ios-preview.sh "samba-phone" --debug
```

The script writes `gen/apple/Preview.xcconfig`, which `Outl.xcconfig` picks up through `#include?`, and passes `src-tauri/tauri.preview.conf.json` to `--config` so the CLI installs and launches the same id it just built.
`app.outl.mobile-app.preview` lands on the Home Screen as **outl Preview**, with its own container, App Group, `outl-preview://` scheme and BGTask ids.
Its icon carries a `PREVIEW` band, so the two are told apart at a glance.

Five things worth knowing before you change it:

- **The script restores `project.pbxproj` and `AppIcon.appiconset` on the way out.**
  Both are committed files the preview has to write, and the CLI is why.
  It rewrites the app target's bundle id in the pbxproj on every run, so a preview otherwise leaves `app.outl.mobile-app.preview` checked in.
  That ships a TestFlight build under the preview id, behind a diff that looks like an unrelated two-line change.
- **The icon is swapped in place, not selected by name.**
  `ASSETCATALOG_COMPILER_APPICON_NAME` pointed at a second `AppIcon-Preview.appiconset` is the clean way to do this, and it does not survive.
  The CLI rewrites that setting back to the literal `AppIcon` too, and the build then succeeds with the production icon and says nothing.
  So `gen-icons.py --preview-appicon` draws the banded slots into a staging directory, the script copies them over `AppIcon.appiconset`, and the originals come back on exit.
  Neither directory is committed, because an appiconset inside `Assets.xcassets` is compiled into **every** build, including the App Store one that can never show it.
- **`Preview.xcconfig` is removed when the script exits**, Ctrl-C included, and is gitignored.
  Preview is a property of that command, not a state the checkout sits in.
  A leftover file would make the next `cargo tauri ios build` produce an IPA that App Store Connect rejects for the wrong bundle id, long after anyone remembers running this.
- **CI never writes it**, which is why the override is a separate file rather than a flag with a default.
  Without it, every setting above resolves to the production value and the build is what it was before this file existed.
- **`--release` on `ios dev` does not change the Xcode configuration.**
  The CLI builds the `debug` configuration either way (it is the Rust profile that changes), which is why the xcconfig is wired to **both** project configurations rather than to `release` alone.

Automatic signing creates the new App ID and App Group on first run, so there is nothing to register by hand.
The extensions are signed the same way.

### Export compliance: the flag is about the paperwork, not about the code

`ITSAppUsesNonExemptEncryption` has three states, and only one of them reads as "do you use encryption":

| state | meaning | upload |
|---|---|---|
| `true` | Apple has **approved encryption documentation** for this app, and its code is in `ITSEncryptionExportComplianceCode` | refused without that code |
| `false` | the encryption is exempt | accepted |
| absent | no claim; App Store Connect asks the export questions **per build** | accepted |

outl declares nothing, which is the honest state: there is no approved documentation, so there is no code to carry.
The cost is one answer per build in App Store Connect, and it is the only state that is true today.

**This was `true` for one build and every upload was refused**, with:

```
Invalid Export Compliance Code. The export compliance key value []
in the app's Info.plist doesn't match the key value of the app's
export compliance documentation.
```

The empty `[]` is the missing **code**, not the missing flag — the flag was present and correct-looking.
Reading it as "the app does not declare encryption" sends you to re-add a key that is already there, which is the wrong half of the pair.

Two things make this expensive to discover, and both are now handled:

- **`altool` exits 0 while refusing.** The upload step greps its log for `ERROR:` for exactly that reason; without that grep the job goes green and nothing reaches TestFlight.
- **The answer arrives after the upload.** A 70MB transfer, then the refusal. `testflight.yml`'s *Check export compliance keys* step reads the pair out of the IPA's `Info.plist` first and fails on the one incoherent combination (`true` with no code), so the loop is seconds instead of twenty minutes.

To leave the undeclared state, set it in **both** `gen/apple/project.yml` and `gen/apple/outl-mobile_iOS/Info.plist` — the Info.plist is committed, so the two must agree:

- **exempt** → `ITSAppUsesNonExemptEncryption: false`
- **documented** → `ITSAppUsesNonExemptEncryption: true` plus `ITSEncryptionExportComplianceCode: <code from App Store Connect>`

What outl ships is iroh/QUIC over rustls + ring, with an ed25519 device identity.
Which export exception that falls under is a regulatory question rather than a code one, which is why neither the project file nor this page answers it.

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
   The pass dials every peer **at once**, not one after another: an unreachable peer costs up to 15s, so a laptop asleep at the head of `peers.json` used to burn the whole 12s refresh window before the always-on peer behind it was dialed.
   Concurrent, a pass costs the slowest single dial instead of the sum.
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

The workspace folder can live in an iCloud container, but **no shipping build offers it**.
A fresh install opens the app-local folder and the onboarding screen has one storage button, "Keep on this device".
The folder picker that would let a user choose iCloud is deferred (`workspace_picker.rs` explains the security-scoped bookmark it needs), so everything in this section and the next applies only to a workspace someone placed in a container by hand.
In that case the root is `<ubiquity-container>/Documents/`.
No Rust code resolves that container: `workspace_open::icloud_workspace_root()` was removed (see `crates/outl-mobile/CLAUDE.md` → "Change detection: the iroh signal").
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
