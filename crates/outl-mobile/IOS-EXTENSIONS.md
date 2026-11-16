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

Control Center controls need iOS 18, but the app and the Home Screen / Lock Screen widget keep the iOS 17 floor.
The control and its intent are `@available(iOS 18.0, *)`, and `OutlWidgetBundle` only lists the control behind `if #available(iOS 18.0, *)`.
Building it needs the iOS 18 SDK (Xcode 16 or later).

There is no audio-capture control: the app has no audio-capture entry point for one to open.

To try it: install the build on an iOS 18 device, open Control Center, tap the `+` in the top left ("Add a Control") and pick "Quick Add" under outl.
Tapping it should open the app on the capture sheet.

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
