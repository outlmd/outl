import Foundation
import OutlKit
import UIKit
import WebKit

/// App side of the capture inbox (RFC 0363).
///
/// `prepare()` runs synchronously in `OutlBootstrap.+load`, before Rust
/// starts: it creates `captures/` in the App Group container and points
/// `OUTL_CAPTURE_INBOX` at it, so `drain_capture_inbox` finds it. Without
/// the App Group entitlement the container is `nil`, the variable stays
/// unset and the drain is a no-op.
///
/// `install()` listens for the Darwin notification an extension posts
/// after writing, and asks the webview to drain
/// (`window.__outlCapturesArrived`, see `@outl/shared/capture`). Launch and
/// foreground drains are driven from the webview itself.
@objc(OutlCaptureInbox)
public final class OutlCaptureInbox: NSObject {

    @objc public static func prepare() {
        guard let inbox = CaptureInbox.shared() else { return }
        try? FileManager.default.createDirectory(at: inbox.directory, withIntermediateDirectories: true)
        setenv(CaptureInbox.envVar, inbox.directory.path, 1)
    }

    @objc public static func install() {
        // The Control Center "Quick Add" intent runs here, in the app,
        // and opens the capture sheet through the same `outl://capture`
        // deep link the widget uses (see `OpenQuickAddIntent.swift`).
        // `main.mm` calls `install()` on the main queue.
        MainActor.assumeIsolated {
            QuickAddRoute.open = { UIApplication.shared.open(CaptureLink.capture) }
        }

        let center = CFNotificationCenterGetDarwinNotifyCenter()
        CFNotificationCenterAddObserver(
            center,
            nil,
            { _, _, _, _, _ in
                DispatchQueue.main.async { OutlCaptureInbox.notifyWebView() }
            },
            CaptureInbox.arrivedNotification as CFString,
            nil,
            .deliverImmediately
        )
    }

    private static func notifyWebView() {
        guard
            let window = OutlWindows.key,
            let web = OutlToolbarView.findWebView(in: window)
        else { return }
        web.evaluateJavaScript(
            "window.__outlCapturesArrived && window.__outlCapturesArrived()",
            completionHandler: nil
        )
    }
}
