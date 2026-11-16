import OutlKit
import UIKit
import WebKit

/// Publishes the real system safe-area insets to the web layer.
///
/// **Why this exists:** the Tauri `WKWebView` fills the screen, so the
/// status bar, the notch and the Dynamic Island sit on top of the page,
/// and it does not hand the page truthful `env(safe-area-inset-*)`
/// values. With the inset reading 0 the header's
/// `max(env(safe-area-inset-top), 12px)` collapsed to its 12px floor and
/// the Journal chrome drew under the clock. We measure the insets here,
/// from UIKit, and the page applies them as `--safe-area-inset-*` on
/// `:root` (`src/lib/safe-area.ts`). No device height is hard-coded:
/// the value is whatever the system reports for this device and
/// orientation.
///
/// **How it follows changes:** an invisible, touch-transparent probe view
/// pinned to the root view overrides `safeAreaInsetsDidChange`, which
/// UIKit calls on rotation, multitasking resizes and
/// `additionalSafeAreaInsets` changes. A `UIWindow` has no such callback
/// of its own.
///
/// **How it survives load order:** the page may boot before or after the
/// first push, so both directions are covered. We push whenever the
/// insets change or the webview appears, and the page can pull by posting
/// to the `outlSafeArea` message handler, which answers with a push.
@objc(OutlSafeArea)
public final class OutlSafeArea: NSObject, WKScriptMessageHandler {

    private static let handlerName = "outlSafeArea"
    private static let shared = OutlSafeArea()

    private var probe: OutlSafeAreaProbe?
    private weak var registeredOn: WKWebView?
    private var retries = 0

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        DispatchQueue.main.async {
            shared.attach()
        }
    }

    private func attach() {
        guard let win = OutlWindows.key,
              let root = win.rootViewController?.view,
              let web = OutlToolbarView.findWebView(in: win)
        else {
            // Window or webview not mounted yet; same bounded polling
            // as `OutlBrandChrome`.
            if retries >= 50 {
                NSLog("[outl] safe area: window/webview never mounted")
                return
            }
            retries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                self.attach()
            }
            return
        }

        if probe?.superview !== root {
            probe?.removeFromSuperview()
            let p = OutlSafeAreaProbe(frame: root.bounds)
            p.onChange = { [weak self] in self?.push() }
            root.addSubview(p)
            probe = p
        }

        if registeredOn !== web {
            registeredOn?.configuration.userContentController
                .removeScriptMessageHandler(forName: Self.handlerName)
            web.configuration.userContentController.add(self, name: Self.handlerName)
            registeredOn = web
        }

        push()
    }

    /// Sends the current insets, in CSS pixels (UIKit points), to the page.
    private func push() {
        guard let probe, let web = registeredOn else { return }
        let i = probe.safeAreaInsets
        let json = String(
            format: "{\"top\":%.2f,\"right\":%.2f,\"bottom\":%.2f,\"left\":%.2f}",
            Double(i.top), Double(i.right), Double(i.bottom), Double(i.left)
        )
        web.evaluateJavaScript(
            "window.__outlSafeArea=\(json);"
                + "window.__outlApplySafeArea&&window.__outlApplySafeArea(window.__outlSafeArea);",
            completionHandler: nil
        )
    }

    // MARK: - WKScriptMessageHandler

    /// The page asking for the insets (it booted after our last push).
    public func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        push()
    }
}

/// Zero-cost view whose only job is to be told when the safe area moves.
private final class OutlSafeAreaProbe: UIView {
    var onChange: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        autoresizingMask = [.flexibleWidth, .flexibleHeight]
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        backgroundColor = .clear
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        onChange?()
    }
}
