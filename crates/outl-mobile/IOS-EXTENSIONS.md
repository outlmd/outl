# iOS app extensions (RFC 0363)

Capture from outside the app: a widget, a Control Center control, an App Intent ("Add to today",
which Shortcuts, Siri and the Action button use) and a share extension.
Design and invariants: [`docs/rfcs/0363-ios-quick-capture.md`](../../docs/rfcs/0363-ios-quick-capture.md).

This file is the **iOS** half. Android has the same three shapes with a
different hand-off and no signing story at all, in
[`docs/android-platform.md`](../../docs/android-platform.md) → Capture
from outside the app.

## What lives where

| Piece | Path | Target |
|---|---|---|
| Widget (opens `outl://capture`) | `src-tauri/gen/apple/OutlWidget/` | `OutlWidget` app extension, iOS 16+ |
| Control Center "Quick Add" control (opens `outl://capture`) | `src-tauri/gen/apple/OutlWidget/OutlQuickAddControl.swift`, intent in `OpenQuickAddIntent.swift` | same `OutlWidget` extension, iOS 18+; the intent is also compiled into the app |
| Capture icon (the mark with a `+`) | `src-tauri/gen/apple/OutlWidget/Assets.xcassets`, written by `scripts/gen-icons.py` | same `OutlWidget` extension |
| Shared pure logic (links, inbox writer, ULID) | `swift/OutlKit/Sources/OutlKit/Capture/` | OutlKit package, unit tested by `swift test` |
| App Intent "Add to today" + App Shortcut | `src-tauri/gen/apple/Sources/outl-mobile/OutlAddToTodayIntent.swift` | app, iOS 16+ (`AppIntents` weak-linked) |
| Share extension (text, one web link) | `src-tauri/gen/apple/OutlShare/` | `OutlShare` app extension, iOS 15+ |
| App bridge (inbox path, arrival notification) | `src-tauri/gen/apple/Sources/outl-mobile/OutlCaptureInbox.swift` | app |
| Drain | `outl_actions::capture`, `outl-tauri-shared/src/capture_inbox.rs` (`drain_capture_inbox`) | Rust, tested on Linux |
| When to drain | `@outl/shared/capture` `installForegroundRefresh` (mount, foreground, `__outlCapturesArrived`) | webview, shared with the desktop |

The `iOS N+` column is the **API** each piece needs, not the floor the app
ships with: the deployment target is **iOS 17**, declared in
`src-tauri/gen/apple/project.yml`, the committed `project.pbxproj`,
`tauri.conf.json` and `swift/OutlKit/Package.swift` (all four, or a build
picks the lowest and an `#available` branch nobody tests comes back).

## Control Center "Quick Add" (iOS 18+)

A `ControlWidget` in the existing `OutlWidget` extension, so it has no target, App ID, profile or App Group of its own.
One tap runs `OpenQuickAddIntent`, which opens the app and routes it to `outl://capture`, the same link the Home Screen widget and Android's quick settings tile open, so the capture sheet appears over today's journal with the keyboard up.
It does not return `OpenURLIntent(CaptureLink.capture)`: from a control, `OpenURLIntent` only opens universal links, and a custom scheme like `outl://` opens nothing on a device (it can appear to work in the simulator).
Instead the intent sets `openAppWhenRun`, and `OpenQuickAddIntent.swift` is compiled into both the `OutlWidget` extension and the app, so `perform()` runs in the app process.
`openAppWhenRun` alone would only resume the last screen, so `perform()` calls `QuickAddRoute`, which the app points at `UIApplication.open(CaptureLink.capture)` in `OutlCaptureInbox.install()`; the URL then takes the same deep-link path as the widget's.
It writes nothing and is a different intent from "Add to today" (which saves a line to the inbox without opening the app), with no Shortcuts phrases.

**Why the first two attempts failed, since both look right.**
A plain `AppIntent` inherits `openAppWhenRun == false`, so it runs inside the widget extension, where an intent it hands back has nobody to open it: the control is a button that does nothing, silently, with no error and no log.
Reaching for `OpenURLIntent` instead looks like the way out, and it is the trap the comment above names, because it opens nothing on a device.
A control's intent has to be compiled into **both** targets, and that is the first thing to check when a control stops working.

Control Center controls need iOS 18, but the app and the Home Screen / Lock Screen widget keep the iOS 17 floor.
The control and its intent are `@available(iOS 18.0, *)`, and `OutlWidgetBundle` only lists the control behind `if #available(iOS 18.0, *)`.
Building it needs the iOS 18 SDK (Xcode 16 or later).

There is no audio-capture control: the app has no audio-capture entry point for one to open.

To try it: install the build on an iOS 18 device, open Control Center, tap the `+` in the top left ("Add a Control") and pick "Quick Add" under outl.
Tapping it should open the app on the capture sheet.

## A widget that skips `containerBackground` goes white

Since the iOS 17 SDK, `containerBackground(for: .widget)` is not styling, it is what lets WidgetKit place the widget on a background at all.
Leave it out and the system does not fall back to its material: it paints an opaque white behind the content.
The tile then reads as a blank card next to every other widget on the Home Screen, and the gallery preview shows nothing at all.
That is what "the widget went white" was, and nothing warns about it, because the widget still builds, still renders its content and still opens its link.

`CaptureWidgetView` declares one per family, and the split matters.
The Home Screen gets the brand gradient; the Lock Screen gets `AccessoryWidgetBackground()`, because an accessory tints its own content and anything opaque there fights the wallpaper instead of sitting on it.

Reproduce it on a simulator rather than guessing: install the app, add the widget from the gallery, and look at it beside Calendar.
No device needed, and the failure is obvious the moment there is something correct next to it.

## The icon is the outl mark, and it has to be an asset

Every capture entry point shows the outl mark with a `+` knocked out of its body, never a system glyph.
`plus.circle` on a Lock Screen control belongs to whichever app the user tapped, and the control is exactly where the brand has to carry itself.

It ships as a **custom SF Symbol** in `OutlWidget/Assets.xcassets`; the widget names it once, in `CaptureWidgetView.icon`.

**The Control Center is the exception, and it is not solved.**
Measured on an iOS 27 simulator, in this order: a `Shape` as the `Label`'s icon renders a `?` placeholder; a PNG imageset renders `?`; the custom SF Symbol loses the `?` — so the Control Center *does* resolve it — and still draws an empty circle.
The same symbol, out of the same `Assets.car`, draws correctly in the widget beside it, so the symbol is good and something about that surface is not.
The control therefore takes `plus` for now, because a blank button is worse than a generic one.
The untested hypothesis is that SpringBoard looks for a control's symbol in the **app's** asset catalog rather than the extension's; moving it there is the next thing to try.

`scripts/gen-icons.py` writes the `.symbolset` from the same `MARK_BODY` geometry every app icon is rasterised from, plus `MARK_PLUS` for the cut, so the widget and the icon beside it on the Home Screen come from one source.
The template is the SF Symbols one cut to what `actool` actually demands.
Four things to get right, and only the first two announce themselves:

1. The glyph is named **`Regular-M`**, not the `Regular-S` the "three required sources" advice leads you to write.
2. The file needs `Baseline-` and `Capline-` guides for **all three** sizes, even though only the M glyph exists.
3. **The geometry carries its own position.** A `transform` on the variant's `<g>` is what the SF Symbols app writes and what `actool` ignores: the symbol compiles, loses its `?` placeholder, and draws nothing, because the path itself still sits at the artboard's origin. `place_path` bakes the scale and offset into the numbers instead.
4. **An arc's two flags stay bare `0` / `1`.** Written as `0.000` by a formatter that treats every number the same, the path still parses in a browser, and CoreSVG rejects it — `actool` prints "CoreSVG has logged an error", **writes the `.car` anyway, and exits 0**. So the build is green and the symbol is blank.

Items 3 and 4 produce the same symptom and neither fails a build, which is why both are written down here.
`xcrun actool --compile` reproduces all four in a second, and that loop is worth using: a full iOS build to see a blank circle costs minutes.
Change the mark there and the widget follows on the next run; there is nothing else to edit.
The colours have a test rather than a generator: they live in `OutlKit`'s `BrandColor` and are pinned by `BrandColorTests`.

The App Group is `group.app.outl.mobile-app`; the inbox is `captures/` in
its container. The app and every extension that writes must carry it.

The extension targets were added to the committed `project.pbxproj` by
`scripts/ios-add-extension.py`; `project.yml` mirrors them for anyone who
regenerates with `xcodegen`.

## What CI checks and what it cannot

- `swift (OutlKit unit tests)` runs on every PR: the pure Swift pieces.
- `iOS Simulator build (opt-in)` (run it from the Actions tab, or label the
  PR `ios-build`) compiles the app with its extensions for the simulator,
  unsigned.
- The release job signs each extension with its own profile, or, when a
  profile secret is missing, unhooks the extensions and ships the app
  alone (`scripts/ios-extensions-signing.py`).

## What needs Xcode on a Mac

1. **Apple Developer portal.** Create the App Group
   `group.app.outl.mobile-app`. Register the App IDs
   `app.outl.mobile-app.widget` and `app.outl.mobile-app.share`. Enable
   App Groups on `app.outl.mobile-app` and on the share App ID, both with
   that group.
2. **Regenerate the app's App Store profile** so it includes the App
   Group, and update `PROVISIONING_PROFILE_BASE64`. The release job drops
   the App Group from the app (and unhooks the extensions) while any
   extension profile secret is missing, so nothing breaks before this.
3. **Extension App Store profiles.** One per extension. Add them as repository
   secrets `WIDGET_PROVISIONING_PROFILE_BASE64` and
   `SHARE_PROVISIONING_PROFILE_BASE64` (base64 of the `.mobileprovision`).
   Until both exist, release builds ship without the extensions.
4. **Run on a device.** Open `src-tauri/gen/apple/outl-mobile.xcodeproj`,
   pick your team on each extension target (Signing & Capabilities), run
   the `outl-mobile_iOS` scheme on a phone, then add the "Add to today"
   widget from the Home Screen and the Lock Screen and tap it: the app
   should open on the capture sheet.
   On iOS 18, add the "Quick Add" control from Control Center too.
5. **Inbox on a device.** With the app in the background, use the
   "Add to today" Shortcut (step 3) or share some text to outl: the line
   should be in today's journal when you open the app, and with the app
   open it should appear within a second (Darwin notification).
