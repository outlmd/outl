import OutlKit
import UIKit
import WebKit

/// Floating UIView added to the key window. Pinned to the top edge of
/// the floating toolbar (`OutlToolbarView`), so the chip strip sits above
/// the pill and never on top of it. Polls
/// `window.__outlSuggesterState` every 150ms while visible, and updates
/// the embedded `OutlSuggestView` accordingly.
///
/// **Position comes from AutoLayout, not keyboard frames.** The toolbar
/// is pinned to `keyboardLayoutGuide`, so anchoring this overlay's bottom
/// to the toolbar's top moves both with the keyboard, with one gap (the
/// toolbar's own top margin, `ToolbarLayout.verticalMargin`) and no
/// arithmetic about what the keyboard frame includes. It used to read
/// `keyboardFrameEnd`, which counted the accessory toolbar; with the
/// accessory gone that math would have put the strip on top of the pill.
///
/// **Z-order:** both are subviews of the key window, and `OutlSwizzle`
/// installs the toolbar first, so this overlay is above it.
///
/// **Why outside the `inputAccessoryView`:** a previous version
/// embedded the chip strip inside the toolbar and the AutoLayout
/// intrinsic-content-size juggling collapsed the whole accessory to
/// 0pt under some keyboard transitions. An overlay in the key window
/// is bulletproof — no AutoLayout reasoner has to figure out two
/// dynamically-sized stacks at the same time.
@objc(OutlSuggestOverlay)
public final class OutlSuggestOverlay: UIView {

    @objc public weak var webView: WKWebView?

    private let suggest = OutlSuggestView()
    private var polling = false
    private var keyboardVisible = false
    private var lastStateSignature: String?

    /// The strip's height: 0 when there is nothing to show, the chip
    /// strip's height otherwise. The only dimension left to update by
    /// hand; position is constraints.
    private lazy var heightConstraint = heightAnchor.constraint(equalToConstant: 0)
    private var windowConstraints: [NSLayoutConstraint] = []

    /// Singleton bound to the key window. Reused across WebView
    /// re-binds (the swizzle calls `installInKeyWindow(webView:)`
    /// every time the WKContentView's `inputAccessoryView` is
    /// re-resolved).
    private static var shared: OutlSuggestOverlay?

    @objc(installInKeyWindowWithWebView:)
    public static func install(webView: WKWebView) {
        guard let win = OutlWindows.key else { return }
        let overlay = shared ?? OutlSuggestOverlay()
        shared = overlay
        overlay.webView = webView
        overlay.suggest.webView = webView
        if overlay.superview !== win {
            overlay.removeFromSuperview()
            win.addSubview(overlay)
            overlay.pin(to: win)
        }
        overlay.startPolling()
        NSLog("[outl] suggest overlay installed in key window")
    }

    public override init(frame: CGRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        heightConstraint.isActive = true
        isHidden = true
        isUserInteractionEnabled = true

        suggest.translatesAutoresizingMaskIntoConstraints = false
        addSubview(suggest)
        NSLayoutConstraint.activate([
            suggest.leadingAnchor.constraint(equalTo: leadingAnchor),
            suggest.trailingAnchor.constraint(equalTo: trailingAnchor),
            suggest.topAnchor.constraint(equalTo: topAnchor),
            suggest.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        let nc = NotificationCenter.default
        nc.addObserver(
            self,
            selector: #selector(keyboardWillShow(_:)),
            name: UIResponder.keyboardWillShowNotification,
            object: nil
        )
        nc.addObserver(
            self,
            selector: #selector(keyboardWillHide(_:)),
            name: UIResponder.keyboardWillHideNotification,
            object: nil
        )
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not supported")
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    /// Pin to the window's sides and to the toolbar's top edge. Without
    /// a toolbar (it is installed first, so this is a defensive branch)
    /// fall back to the keyboard's top, offset by the toolbar's height,
    /// which is the same place.
    private func pin(to window: UIWindow) {
        NSLayoutConstraint.deactivate(windowConstraints)
        let bottom: NSLayoutConstraint
        if let toolbar = OutlToolbarView.installed {
            bottom = bottomAnchor.constraint(equalTo: toolbar.topAnchor)
        } else {
            bottom = bottomAnchor.constraint(
                equalTo: window.keyboardLayoutGuide.topAnchor,
                constant: -CGFloat(ToolbarLayout.stackHeight)
            )
        }
        windowConstraints = [
            leadingAnchor.constraint(equalTo: window.leadingAnchor),
            trailingAnchor.constraint(equalTo: window.trailingAnchor),
            bottom,
        ]
        NSLayoutConstraint.activate(windowConstraints)
    }

    // MARK: - Keyboard notifications

    @objc private func keyboardWillShow(_ note: Notification) {
        keyboardVisible = true
        updateHeight()
    }

    @objc private func keyboardWillHide(_ note: Notification) {
        keyboardVisible = false
        suggest.hide()
        heightConstraint.constant = 0
        isHidden = true
    }

    /// Size the strip to the chips. Position needs no update: the
    /// constraints in `pin(to:)` already keep the bottom on the pill.
    private func updateHeight() {
        let height = suggest.intrinsicContentSize.height
        heightConstraint.constant = height
        isHidden = (height == 0) || !keyboardVisible
    }

    // MARK: - Polling

    private func startPolling() {
        guard !polling else { return }
        polling = true
        lastStateSignature = nil
        pollOnce()
    }

    private func pollOnce() {
        guard polling else { return }
        guard let web = webView else {
            // WebView not bound yet — retry after a beat so the swizzle
            // has time to bind us via `install(webView:)`.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                self?.pollOnce()
            }
            return
        }
        let js = "JSON.stringify(window.__outlSuggesterState || null)"
        web.evaluateJavaScript(js) { [weak self] result, _ in
            guard let self, self.polling else { return }
            if let json = result as? String {
                self.applyState(json: json)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
                self?.pollOnce()
            }
        }
    }

    private func applyState(json: String) {
        // Signature dedup avoids re-rendering when the JS payload
        // hasn't actually changed (every 150ms is a lot of churn).
        if json == lastStateSignature { return }
        lastStateSignature = json

        let state = SuggesterState.parse(jsonString: json)
        switch state.action {
        case .show:
            // Re-render via the typed parser → re-encoded raw form so
            // we keep one source of truth for "what counts as a valid
            // chip". The cost is a round trip through a dictionary,
            // which is negligible at 150ms polling.
            let raw = state.items.map { item -> [String: Any] in
                [
                    "title": item.title,
                    "slug": item.slug,
                    "kind": item.kind.rawValue,
                ]
            }
            suggest.show(rawItems: raw)
        case .hide:
            suggest.hide()
        }
        updateHeight()
    }
}
