import OutlKit
import UIKit
import WebKit

/// Keyboard accessory toolbar shown above the iOS keyboard while a
/// block is being edited.
///
/// Bear-style: a single rounded-full pill that floats over the
/// keyboard, no dividers, no edge-to-edge bar. Buttons re-order by
/// usage — `OutlKit.ToolbarMFU` owns the persistence + ordering
/// algorithm (and its unit tests). This file only does the UIKit
/// rendering and the JS bridge.
///
/// Exposed to `main.mm` / `OutlSwizzle` via `@objc(OutlToolbarView)`;
/// the swizzle instantiates this and returns it as the
/// `inputAccessoryView` for the `WKContentView`.
@objc(OutlToolbarView)
public final class OutlToolbarView: UIView {

    // MARK: - Public API used by Obj-C / OutlSwizzle

    @objc public weak var webView: WKWebView?

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
        let resolved = frame == .zero
            ? CGRect(x: 0, y: 0, width: 0, height: 56)
            : frame
        super.init(frame: resolved)
        backgroundColor = .clear
        autoresizingMask = .flexibleWidth
        setupViews()
        rebuildButtons()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not supported")
    }

    public override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: 56)
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
            capsule.heightAnchor.constraint(equalToConstant: 44),

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
        // 3. Middle (MFU-reordered, pinned slots excluded).
        for action in ToolbarMFU.orderedMiddleActions() {
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
        ToolbarMFU.record(action)
        invoke(action)
        DispatchQueue.main.async { [weak self] in self?.rebuildButtons() }
    }

    private func invoke(_ action: ToolbarAction) {
        guard let web = resolveWebView() else { return }
        let escaped = JSStringEscape.singleQuoted(action.rawValue)
        let js = "window.__outlToolbar && window.__outlToolbar('\(escaped)')"
        web.evaluateJavaScript(js, completionHandler: nil)
    }

    private func resolveWebView() -> WKWebView? {
        if let w = webView { return w }
        var window: UIWindow?
        if let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene {
            if #available(iOS 15.0, *) {
                window = scene.keyWindow
            }
            if window == nil {
                window = scene.windows.first
            }
        }
        if window == nil {
            window = UIApplication.shared.windows.first
        }
        let web = Self.findWebView(in: window)
        webView = web
        return web
    }
}
