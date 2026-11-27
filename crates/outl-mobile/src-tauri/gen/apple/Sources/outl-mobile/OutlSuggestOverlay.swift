import OutlKit
import UIKit
import WebKit

/// Floating UIView added to the key window. Listens to keyboard
/// notifications to position itself flush above the keyboard, polls
/// `window.__outlSuggesterState` every 150ms while visible, and
/// updates the embedded `OutlSuggestView` accordingly.
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
    private var keyboardFrame: CGRect = .zero
    private var lastStateSignature: String?

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
        }
        overlay.startPolling()
        NSLog("[outl] suggest overlay installed in key window")
    }

    public override init(frame: CGRect) {
        super.init(frame: frame)
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
            selector: #selector(keyboardWillChange(_:)),
            name: UIResponder.keyboardWillChangeFrameNotification,
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

    // MARK: - Keyboard notifications

    @objc private func keyboardWillShow(_ note: Notification) {
        keyboardVisible = true
        if let frame = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue {
            keyboardFrame = frame
        }
        layoutAboveKeyboard()
    }

    @objc private func keyboardWillChange(_ note: Notification) {
        guard keyboardVisible else { return }
        if let frame = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue {
            keyboardFrame = frame
        }
        layoutAboveKeyboard()
    }

    @objc private func keyboardWillHide(_ note: Notification) {
        keyboardVisible = false
        suggest.hide()
        isHidden = true
    }

    private func layoutAboveKeyboard() {
        guard let win = superview as? UIWindow else { return }
        // `UIKeyboardFrameEndUserInfoKey` already accounts for the
        // `inputAccessoryView` (the formatting toolbar). `kbTop` is
        // the top edge of that combined unit. We just sit flush above.
        let kbTop = keyboardFrame.origin.y
        let overlayHeight = suggest.intrinsicContentSize.height
        frame = CGRect(
            x: 0,
            y: kbTop - overlayHeight,
            width: win.bounds.width,
            height: overlayHeight
        )
        isHidden = (overlayHeight == 0)
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
        layoutAboveKeyboard()
    }
}
