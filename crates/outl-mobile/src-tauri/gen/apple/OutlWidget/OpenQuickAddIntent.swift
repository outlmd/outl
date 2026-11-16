import AppIntents
import Foundation

/// The intent behind the Control Center "Quick Add" control (iOS 18+).
///
/// This file is compiled into **both** the `OutlWidget` extension (so the
/// control can name the intent) and the app (so the system can run it
/// there): with `openAppWhenRun`, `perform()` runs in the app process,
/// after the app is in the foreground.
///
/// It does not return `OpenURLIntent(CaptureLink.capture)`: from a
/// control, `OpenURLIntent` only opens universal links, and `outl://` is
/// a custom scheme, so the tap would open nothing on a device.
/// `openAppWhenRun` alone only resumes the last screen, so `perform()`
/// asks the app to route to the capture sheet through `QuickAddRoute`.
@available(iOS 18.0, *)
struct OpenQuickAddIntent: AppIntent {
    static let title: LocalizedStringResource = "Quick Add"
    static let description = IntentDescription("Open outl on a new line in today's journal.")
    static let openAppWhenRun = true
    /// Control Center only; "Add to today" is the Shortcuts entry point.
    static let isDiscoverable = false

    @MainActor
    func perform() async throws -> some IntentResult {
        QuickAddRoute.request()
        return .result()
    }
}

/// How the app opens the capture sheet for `OpenQuickAddIntent`.
///
/// The app sets `open` at launch (`OutlCaptureInbox.install`); the widget
/// extension never sets it and never runs `perform()`. A request that
/// arrives before `open` is set is held and fired once it is.
@MainActor
enum QuickAddRoute {
    static var open: (@MainActor () -> Void)? {
        didSet {
            guard pending, let open else { return }
            pending = false
            open()
        }
    }

    private static var pending = false

    static func request() {
        if let open {
            open()
        } else {
            pending = true
        }
    }
}
