import OutlKit
import UIKit
import WebKit

/// Floating toolbar shown above the iOS keyboard while a block is being
/// edited.
///
/// Bear-style: a single rounded-full pill, no dividers, no edge-to-edge
/// bar. It is **not** an `inputAccessoryView`. An accessory lives in the
/// keyboard's own window, on a host that defaults to solid black, so a
/// pill that floats over a clear strip there shows that black around it
/// (and painting the host grey only trades one slab for another). This
/// view is a full-width, fully transparent overlay in the key window,
/// pinned to `keyboardLayoutGuide.topAnchor`, the same lifetime idea as
/// `OutlSuggestOverlay` (a singleton, rebound when the webview is
/// bound). Only the capsule takes touches: `hitTest` hands everything
/// else back to the page, so scrolling and caret taps work right up to
/// the pill's edge.
///
/// `OutlSwizzle` still replaces `WKContentView.inputAccessoryView`, but
/// with a zero-height stub, only to keep the system form bar
/// (`up/down/done`) away.
///
/// Because the keyboard no longer carries the pill, the page's viewport
/// shrinks to the keys alone. `ToolbarLayout.showClearanceScript` tells
/// the page how much of its bottom edge the pill covers while it is up
/// (`--outl-toolbar-clearance`, consumed as `scroll-padding-bottom`), and
/// `hideClearanceScript` takes it back when the keyboard goes.
///
/// Buttons re-order by usage: `OutlKit.ToolbarMFU` owns the ordering
/// algorithm, `OutlKit.ToolbarLock` owns the user's opt-out from it (and
/// both own their unit tests). This file only does the UIKit rendering
/// and the JS bridge.
///
/// **The row is rebuilt when the keyboard *appears*, never on a tap
/// and never while it is already up.** `OutlSwizzle` keeps one
/// instance for the whole app lifetime, so a keyboard notification is
/// what marks the start of an editing session. Rebuilding inside
/// `handle` (which is what this did) moved the button the user had
/// just hit out from under their finger before they could hit it
/// again — indent, indent, indent landed on three different buttons,
/// issue #269.
///
/// `keyboardWillShowNotification` is **not** one-per-session: iOS
/// re-posts it while the keyboard is on screen (input-mode switch,
/// emoji, the QuickType bar appearing — which this app keeps on).
/// Acting on every post would put the mid-session reshuffle straight
/// back. `keyboardVisible` reduces the stream to its rising edge, the
/// same guard `OutlSuggestOverlay` already keeps for the same reason.
///
/// **Tap counts live in the webview's `localStorage`, not
/// `UserDefaults`.** The bar is native but the settings sheet that
/// reads those counts is web, so a `UserDefaults` store would be
/// invisible to it: "Lock button order" would freeze a cold-start row
/// instead of the user's, and "Reset button order" would do nothing.
/// `window.__outlToolbar(action)` records the tap on the JS side, next
/// to the dispatch, which makes `Journal.tsx` the single counter for
/// both bars.
///
/// Exposed to `OutlSwizzle` via `@objc(OutlToolbarView)`; the swizzle
/// installs it into the key window once the webview exists.
@objc(OutlToolbarView)
public final class OutlToolbarView: UIView {

    // MARK: - Public API used by Obj-C / OutlSwizzle

    @objc public weak var webView: WKWebView?

    /// Singleton bound to the key window. Reused across re-binds.
    private static var shared: OutlToolbarView?

    /// The installed overlay, so `OutlSuggestOverlay` can pin itself to
    /// the pill's top edge instead of guessing at the keyboard's.
    static var installed: OutlToolbarView? { shared }

    /// Put the toolbar in the key window and bind it to `webView`.
    /// Idempotent: a second call only rebinds.
    @objc(installInKeyWindowWithWebView:)
    public static func install(webView: WKWebView) {
        guard let win = OutlWindows.key else { return }
        let bar = shared ?? OutlToolbarView()
        shared = bar
        bar.webView = webView
        if bar.superview !== win {
            bar.removeFromSuperview()
            win.addSubview(bar)
            bar.pin(to: win)
        }
        NSLog("[outl] floating toolbar installed in key window")
    }

    @objc(findWebViewIn:)
    public static func findWebView(in view: UIView?) -> WKWebView? {
        guard let view else { return nil }
        if let web = view as? WKWebView { return web }
        for sub in view.subviews {
            if let found = findWebView(in: sub) { return found }
        }
        return nil
    }

    // MARK: - Per-action presentation metadata

    private enum Style {
        case symbol(String, destructive: Bool)
        case text(String)
    }

    private struct ActionMeta {
        let label: String
        let style: Style
    }

    private static let metadata: [ToolbarAction: ActionMeta] = [
        .newLine:     ActionMeta(label: "New line",         style: .symbol("plus", destructive: false)),
        .indent:      ActionMeta(label: "Indent",           style: .symbol("increase.indent", destructive: false)),
        .outdent:     ActionMeta(label: "Outdent",          style: .symbol("decrease.indent", destructive: false)),
        .moveUp:      ActionMeta(label: "Move up",          style: .symbol("arrow.up", destructive: false)),
        .moveDown:    ActionMeta(label: "Move down",        style: .symbol("arrow.down", destructive: false)),
        .undo:        ActionMeta(label: "Undo",             style: .symbol("arrow.uturn.left", destructive: false)),
        .redo:        ActionMeta(label: "Redo",             style: .symbol("arrow.uturn.right", destructive: false)),
        .bold:        ActionMeta(label: "Bold",             style: .symbol("bold", destructive: false)),
        .italic:      ActionMeta(label: "Italic",           style: .symbol("italic", destructive: false)),
        .code:        ActionMeta(label: "Code",             style: .symbol("chevron.left.forwardslash.chevron.right", destructive: false)),
        .insertRef:   ActionMeta(label: "Insert reference", style: .text("[[")),
        .insertBlock: ActionMeta(label: "Insert block ref", style: .text("((")),
        .insertHash:  ActionMeta(label: "Insert hashtag",   style: .text("#")),
        .toggleTodo:  ActionMeta(label: "Toggle TODO",      style: .symbol("checkmark.circle", destructive: false)),
        .delete:      ActionMeta(label: "Delete block",     style: .symbol("trash", destructive: true)),
        .done:        ActionMeta(label: "Hide keyboard",    style: .symbol("keyboard.chevron.compact.down", destructive: false)),
    ]

    // MARK: - Order state

    /// The frozen order the user locked, or `nil` when the toolbar is
    /// free to follow MFU. Cached from the webview's `localStorage`
    /// (see `refreshOrderState`) because reading it is asynchronous
    /// and `rebuildButtons` has to be able to run synchronously — at
    /// init there is no webview bound yet.
    private var lockedOrder: [ToolbarAction]?

    /// Tap counts, cached from the same read as `lockedOrder`. Empty
    /// until the first successful read, which is why the very first
    /// build shows the cold-start order.
    private var counts: [String: Int] = [:]

    /// Whether the keyboard is currently on screen. Gates
    /// `keyboardWillShow` down to its rising edge — see the type's doc
    /// comment for why the raw notification is not a session boundary.
    private var keyboardVisible = false

    /// Whether the pill is currently presented (or on its way in).
    /// Distinct from `keyboardVisible`: a keyboard raised for a sheet
    /// field is up with no pill.
    private var pillShown = false

    /// Constraints that pin the overlay to its window; replaced when the
    /// window changes.
    private var windowConstraints: [NSLayoutConstraint] = []

    // MARK: - Subviews

    /// Visual capsule background — single rounded pill behind
    /// `leftPinned`, `scroll`, and `rightPinned` so the three areas
    /// read as one toolbar.
    private let capsule = UIView()
    /// Always-visible holder for `ToolbarAction.pinnedFirst` (`.newLine`).
    /// Lives on the left edge of the capsule, outside the scroll view,
    /// so the `+` button doesn't drift off-screen when the user scrolls
    /// the middle row.
    private let leftPinned = UIStackView()
    /// Always-visible holder for `ToolbarAction.pinnedLast` (`.done`).
    /// Mirrors `leftPinned` on the right edge — "Hide keyboard" stays
    /// reachable from the thumb's resting position regardless of how
    /// far the scroll moved.
    private let rightPinned = UIStackView()
    /// Horizontal scroll for the MFU-reordered middle range of actions.
    /// Pinned actions are deliberately **outside** this view so they
    /// stay visible at the edges; only the middle scrolls.
    private let scroll = UIScrollView()
    /// Inner stack of the scroll — populated with the middle actions
    /// (everything that isn't `pinnedFirst` / `pinnedLast`).
    private let stack = UIStackView()

    // MARK: - Init

    public override init(frame: CGRect) {
        super.init(frame: .zero)
        // Truly transparent: nothing but the capsule paints, so the
        // page shows around it (see the type's doc comment).
        backgroundColor = .clear
        isOpaque = false
        translatesAutoresizingMaskIntoConstraints = false
        isHidden = true
        alpha = 0
        heightAnchor.constraint(
            equalToConstant: CGFloat(ToolbarLayout.stackHeight)
        ).isActive = true
        setupViews()
        rebuildButtons()
        // A new editing session starts when the keyboard appears. This
        // is the only place the row is allowed to change shape.
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

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not supported")
    }

    /// Anchor to the window's safe-area edges horizontally (clean in
    /// portrait and landscape) and to the top of the keyboard vertically.
    /// `keyboardLayoutGuide` moves with the keyboard, so the pill rides
    /// the keyboard's animation without any frame bookkeeping here.
    private func pin(to window: UIWindow) {
        NSLayoutConstraint.deactivate(windowConstraints)
        windowConstraints = [
            leadingAnchor.constraint(equalTo: window.safeAreaLayoutGuide.leadingAnchor),
            trailingAnchor.constraint(equalTo: window.safeAreaLayoutGuide.trailingAnchor),
            bottomAnchor.constraint(equalTo: window.keyboardLayoutGuide.topAnchor),
        ]
        NSLayoutConstraint.activate(windowConstraints)
    }

    // MARK: - Hit testing

    /// Only the capsule takes touches. The container is full width and
    /// transparent, so a hit that lands on it (and not on the capsule or
    /// a button) belongs to the page underneath: returning `nil` lets
    /// UIKit move on to the webview, which keeps scroll, taps and the
    /// caret working beside and around the pill.
    public override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit === self ? nil : hit
    }

    // MARK: - Layout

    private func setupViews() {
        capsule.translatesAutoresizingMaskIntoConstraints = false
        capsule.layer.cornerRadius = 22
        // The capsule deliberately does NOT clip — `clipsToBounds`
        // and `masksToBounds` are the same flag, so enabling either
        // would also clip the drop shadow. The scrollable middle
        // already clips its own content (`scroll.clipsToBounds`); the
        // pinned containers live outside the scroll and have nothing
        // to overflow.
        capsule.backgroundColor = UIColor { trait in
            trait.userInterfaceStyle == .dark
                ? UIColor(white: 0.18, alpha: 0.98)
                : UIColor(white: 1.0, alpha: 0.98)
        }
        capsule.layer.shadowColor = UIColor.black.cgColor
        capsule.layer.shadowOffset = CGSize(width: 0, height: 2)
        capsule.layer.shadowRadius = 8
        capsule.layer.shadowOpacity = 0.12
        addSubview(capsule)

        // ── Pinned containers ──────────────────────────────────────
        // Both stacks hold exactly one button each (the pinned action),
        // but UIStackView gives us free centering + the same spacing
        // rules as the middle stack so the visual rhythm matches.
        for pinned in [leftPinned, rightPinned] {
            pinned.axis = .horizontal
            pinned.alignment = .center
            pinned.spacing = 0
            pinned.translatesAutoresizingMaskIntoConstraints = false
            capsule.addSubview(pinned)
        }

        // ── Scrollable middle ─────────────────────────────────────
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.showsHorizontalScrollIndicator = false
        scroll.showsVerticalScrollIndicator = false
        scroll.alwaysBounceHorizontal = false
        scroll.alwaysBounceVertical = false
        scroll.bounces = false
        scroll.scrollsToTop = false
        // No cornerRadius / clipsToBounds here: the scroll lives in the
        // middle of the capsule, never touches the rounded corners. The
        // capsule itself clips so middle-row content can't bleed under
        // the pinned buttons during a scroll-overshoot.
        scroll.clipsToBounds = true
        capsule.addSubview(scroll)

        stack.axis = .horizontal
        stack.alignment = .center
        stack.spacing = 4
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)

        NSLayoutConstraint.activate([
            capsule.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            capsule.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            capsule.centerYAnchor.constraint(equalTo: centerYAnchor),
            capsule.heightAnchor.constraint(
                equalToConstant: CGFloat(ToolbarLayout.capsuleHeight)
            ),

            // Pinned left: flush against the capsule's left edge.
            leftPinned.leadingAnchor.constraint(equalTo: capsule.leadingAnchor, constant: 8),
            leftPinned.topAnchor.constraint(equalTo: capsule.topAnchor),
            leftPinned.bottomAnchor.constraint(equalTo: capsule.bottomAnchor),

            // Pinned right: flush against the capsule's right edge.
            rightPinned.trailingAnchor.constraint(equalTo: capsule.trailingAnchor, constant: -8),
            rightPinned.topAnchor.constraint(equalTo: capsule.topAnchor),
            rightPinned.bottomAnchor.constraint(equalTo: capsule.bottomAnchor),

            // Scroll sits between the two pinned stacks. The `4pt`
            // gap on each side keeps the first/last scrolled buttons
            // from kissing the pinned buttons during a scroll.
            scroll.leadingAnchor.constraint(equalTo: leftPinned.trailingAnchor, constant: 4),
            scroll.trailingAnchor.constraint(equalTo: rightPinned.leadingAnchor, constant: -4),
            scroll.topAnchor.constraint(equalTo: capsule.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: capsule.bottomAnchor),

            stack.leadingAnchor.constraint(equalTo: scroll.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: scroll.trailingAnchor),
            stack.topAnchor.constraint(equalTo: scroll.topAnchor),
            stack.bottomAnchor.constraint(equalTo: scroll.bottomAnchor),
            stack.heightAnchor.constraint(equalTo: scroll.heightAnchor),
        ])
    }

    private func rebuildButtons() {
        // 1. Clear all three containers.
        for container in [leftPinned, stack, rightPinned] {
            for view in container.arrangedSubviews {
                container.removeArrangedSubview(view)
                view.removeFromSuperview()
            }
        }
        // 2. Pinned left (`pinnedFirst` = `.newLine`).
        leftPinned.addArrangedSubview(makeButton(for: ToolbarAction.pinnedFirst))
        // 3. Middle (frozen order when locked, else MFU-reordered;
        //    pinned slots excluded from both).
        let middle = ToolbarLock.resolveMiddleActions(
            counts: counts,
            locked: lockedOrder
        )
        for action in middle {
            stack.addArrangedSubview(makeButton(for: action))
        }
        // 4. Pinned right (`pinnedLast` = `.done`, "Hide keyboard").
        rightPinned.addArrangedSubview(makeButton(for: ToolbarAction.pinnedLast))
    }

    private func makeButton(for action: ToolbarAction) -> UIButton {
        guard let meta = Self.metadata[action] else {
            return UIButton(type: .system)
        }
        let btn = UIButton(type: .system)
        switch meta.style {
        case .symbol(let name, let destructive):
            let cfg = UIImage.SymbolConfiguration(pointSize: 18, weight: .regular)
            let img = UIImage(systemName: name, withConfiguration: cfg)?
                .withRenderingMode(.alwaysTemplate)
            btn.setImage(img, for: .normal)
            btn.tintColor = destructive ? .systemRed : .label
        case .text(let title):
            btn.setTitle(title, for: .normal)
            btn.setTitleColor(.label, for: .normal)
            btn.titleLabel?.font = .monospacedSystemFont(ofSize: 16, weight: .medium)
        }
        btn.accessibilityLabel = meta.label
        btn.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            btn.widthAnchor.constraint(equalToConstant: 40),
            btn.heightAnchor.constraint(equalToConstant: 38),
        ])
        btn.addAction(
            UIAction { [weak self] _ in self?.handle(action) },
            for: .touchUpInside
        )
        return btn
    }

    // MARK: - Tap handling

    private func handle(_ action: ToolbarAction) {
        // No count written here and no `rebuildButtons()` — see the
        // type's doc comment. `invoke` hands the action to JS, which
        // records the tap into the same `localStorage` the settings
        // sheet reads; the layout follows at the next keyboard
        // appearance.
        invoke(action)
    }

    // MARK: - Session boundary

    /// Start of an editing session: re-read the stored order state,
    /// then lay the row out once. Everything that can move the buttons
    /// happens here and nowhere else.
    ///
    /// Guarded on the rising edge: iOS re-posts this while the keyboard
    /// is already up, and rebuilding then is the mid-session reshuffle
    /// issue #269 was about. The same edge decides whether the pill
    /// shows at all: only for the editor, never for a sheet's field.
    @objc private func keyboardWillShow(_ note: Notification) {
        guard !keyboardVisible else { return }
        keyboardVisible = true
        let timing = KeyboardTiming(note)
        refreshOrderState { [weak self] in
            guard let self, self.keyboardVisible else { return }
            self.rebuildButtons()
            self.presentIfEditing(timing)
        }
    }

    @objc private func keyboardWillHide(_ note: Notification) {
        keyboardVisible = false
        dismiss(KeyboardTiming(note))
    }

    // MARK: - Presentation

    /// Show the pill when the keyboard belongs to the editor: the
    /// webview's content view holds first responder (not a native field
    /// in a sheet) and the focused element is the outline's textarea
    /// (not a field in a web sheet). A probe that fails to answer shows
    /// the pill, see `ToolbarLayout.isEditorFocused`.
    private func presentIfEditing(_ timing: KeyboardTiming) {
        guard let web = resolveWebView(), Self.holdsFirstResponder(web) else { return }
        web.evaluateJavaScript(ToolbarLayout.editorFocusedScript) { [weak self] result, _ in
            guard let self, self.keyboardVisible, !self.pillShown,
                  ToolbarLayout.isEditorFocused(result)
            else { return }
            self.present(timing)
        }
    }

    private func present(_ timing: KeyboardTiming) {
        pillShown = true
        // No `bringSubviewToFront`: the suggester overlay is added after
        // this view and has to stay above it (see `OutlSwizzle.bindWebView`).
        isHidden = false
        webView?.evaluateJavaScript(ToolbarLayout.showClearanceScript, completionHandler: nil)
        UIView.animate(
            withDuration: timing.duration,
            delay: 0,
            options: [timing.curve, .beginFromCurrentState]
        ) {
            self.alpha = 1
        }
    }

    private func dismiss(_ timing: KeyboardTiming) {
        guard pillShown else { return }
        pillShown = false
        webView?.evaluateJavaScript(ToolbarLayout.hideClearanceScript, completionHandler: nil)
        UIView.animate(
            withDuration: timing.duration,
            delay: 0,
            options: [timing.curve, .beginFromCurrentState],
            animations: { self.alpha = 0 },
            completion: { [weak self] _ in
                // A new session may have started during the fade; its
                // `present` owns visibility from then on.
                guard let self, !self.pillShown else { return }
                self.isHidden = true
            }
        )
    }

    /// Is some view inside `web` the first responder? `WKContentView` is
    /// private, so this asks the hierarchy rather than naming it.
    private static func holdsFirstResponder(_ web: UIView) -> Bool {
        if web.isFirstResponder { return true }
        return web.subviews.contains { holdsFirstResponder($0) }
    }

    /// The keyboard's own animation parameters, so the pill fades on the
    /// same clock as the keys instead of a made-up one.
    private struct KeyboardTiming {
        let duration: TimeInterval
        let curve: UIView.AnimationOptions

        init(_ note: Notification) {
            let info = note.userInfo
            duration = (info?[UIResponder.keyboardAnimationDurationUserInfoKey] as? NSNumber)?
                .doubleValue ?? 0.25
            let raw = (info?[UIResponder.keyboardAnimationCurveUserInfoKey] as? NSNumber)?
                .uintValue ?? UInt(UIView.AnimationCurve.easeInOut.rawValue)
            curve = UIView.AnimationOptions(rawValue: raw << 16)
        }
    }

    /// Pull the tap counts and the lock out of the webview's
    /// `localStorage`, which is where both live (`OutlKit.ToolbarLock`
    /// explains why the bar does not keep its own `UserDefaults` copy).
    /// One `evaluateJavaScript` for both, so the two can't be read from
    /// different moments.
    ///
    /// Async because `evaluateJavaScript` is; the keyboard animation
    /// gives it far more time than it needs, and a failure (JS not
    /// ready, storage disabled) leaves the previous values in place
    /// rather than silently unlocking a bar the user locked.
    ///
    /// Resolves the webview through `resolveWebView()`, not the stored
    /// property: `OutlSwizzle.bindWebView` gives up after ~4s, and
    /// `invoke` already re-scans the hierarchy for exactly that reason.
    /// Reading it the other way meant a bound-late webview left the bar
    /// ignoring the user's lock with no error anywhere.
    private func refreshOrderState(completion: @escaping () -> Void) {
        guard let web = resolveWebView() else {
            completion()
            return
        }
        let js = """
        JSON.stringify([
          localStorage.getItem('\(ToolbarMFU.storageKey)'),
          localStorage.getItem('\(ToolbarLock.storageKey)')
        ])
        """
        web.evaluateJavaScript(js) { [weak self] result, error in
            guard let self else {
                completion()
                return
            }
            if error == nil, let pair = ToolbarStore.parsePair(result as? String) {
                // A missing key parses to "no counts" / "not locked",
                // which are the correct answers and the one case where
                // clearing the cache is right.
                self.counts = pair.counts
                self.lockedOrder = pair.locked
            }
            completion()
        }
    }

    private func invoke(_ action: ToolbarAction) {
        guard let web = resolveWebView() else { return }
        let escaped = JSStringEscape.singleQuoted(action.rawValue)
        let js = "window.__outlToolbar && window.__outlToolbar('\(escaped)')"
        web.evaluateJavaScript(js, completionHandler: nil)
    }

    private func resolveWebView() -> WKWebView? {
        if let w = webView { return w }
        let web = Self.findWebView(in: OutlWindows.key)
        webView = web
        return web
    }
}
