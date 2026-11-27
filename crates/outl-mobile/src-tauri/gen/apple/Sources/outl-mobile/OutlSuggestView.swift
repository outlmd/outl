import OutlKit
import UIKit
import WebKit

/// Floating chip strip rendered when the user is typing inside `[[…]]`.
///
/// Sits inside an `OutlSuggestOverlay` which positions it flush above
/// the keyboard. Each chip carries a `slug` + `kind` payload that
/// fires `window.__outlSuggesterPicked(slug, kind)` on tap; the JS
/// layer then synchronously rewrites the textarea so the keyboard
/// stays up.
///
/// Visual: matches the iOS QuickType strip (36pt, system font,
/// vertical hairline dividers, dynamic dark/light) so users read it
/// as native chrome rather than an app overlay.
@objc(OutlSuggestView)
public final class OutlSuggestView: UIView {

    @objc public weak var webView: WKWebView?

    private let scroll = UIScrollView()
    private let chipStack = UIStackView()

    /// `intrinsicContentSize` collapses to zero when there's nothing
    /// to show. The overlay relies on this to slide off-screen rather
    /// than render an empty bar.
    private var visible = false

    public override init(frame: CGRect) {
        super.init(frame: frame)
        // Match the formatting toolbar's old background so when both
        // were stacked they read as one continuous slab. With the new
        // capsule toolbar this is less load-bearing, but the QuickType-
        // adjacent color still feels right.
        backgroundColor = UIColor { trait in
            trait.userInterfaceStyle == .dark
                ? UIColor(white: 0.11, alpha: 1.0)
                : UIColor(white: 0.97, alpha: 1.0)
        }

        let topLine = UIView()
        topLine.translatesAutoresizingMaskIntoConstraints = false
        topLine.backgroundColor = UIColor { trait in
            trait.userInterfaceStyle == .dark
                ? UIColor(white: 1.0, alpha: 0.10)
                : UIColor(white: 0.0, alpha: 0.10)
        }
        addSubview(topLine)

        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.showsHorizontalScrollIndicator = false
        scroll.alwaysBounceHorizontal = true
        scroll.contentInset = .zero
        addSubview(scroll)

        chipStack.axis = .horizontal
        chipStack.alignment = .center
        chipStack.spacing = 4
        chipStack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(chipStack)

        NSLayoutConstraint.activate([
            topLine.leadingAnchor.constraint(equalTo: leadingAnchor),
            topLine.trailingAnchor.constraint(equalTo: trailingAnchor),
            topLine.topAnchor.constraint(equalTo: topAnchor),
            topLine.heightAnchor.constraint(equalToConstant: 0.5),

            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: topAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),

            chipStack.leadingAnchor.constraint(equalTo: scroll.leadingAnchor, constant: 8),
            chipStack.trailingAnchor.constraint(equalTo: scroll.trailingAnchor, constant: -8),
            chipStack.topAnchor.constraint(equalTo: scroll.topAnchor),
            chipStack.bottomAnchor.constraint(equalTo: scroll.bottomAnchor),
            chipStack.heightAnchor.constraint(equalTo: scroll.heightAnchor),
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not supported")
    }

    public override var intrinsicContentSize: CGSize {
        // 36pt is the iOS QuickType strip height — visually familiar.
        CGSize(width: UIView.noIntrinsicMetric, height: visible ? 36 : 0)
    }

    /// Render `items` as chips. Pass an empty array to collapse.
    /// Public for Obj-C so `OutlSuggestOverlay` can call it from
    /// either Swift or Obj-C call sites during the port.
    @objc public func show(rawItems: [[String: Any]]) {
        let parsed = ChipItemParser.parse(rawItems)
        renderChips(parsed)
    }

    @objc public func hide() {
        renderChips([])
    }

    private func renderChips(_ items: [ChipItem]) {
        for view in chipStack.arrangedSubviews {
            chipStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        for (i, item) in items.enumerated() {
            if i > 0 {
                chipStack.addArrangedSubview(makeDivider())
            }
            chipStack.addArrangedSubview(makeChip(for: item))
        }
        let wantVisible = !items.isEmpty
        if wantVisible != visible {
            visible = wantVisible
            invalidateIntrinsicContentSize()
            superview?.invalidateIntrinsicContentSize()
            superview?.setNeedsLayout()
        }
    }

    private func makeDivider() -> UIView {
        let line = UIView()
        line.translatesAutoresizingMaskIntoConstraints = false
        line.backgroundColor = UIColor { trait in
            trait.userInterfaceStyle == .dark
                ? UIColor(white: 1.0, alpha: 0.15)
                : UIColor(white: 0.0, alpha: 0.15)
        }
        NSLayoutConstraint.activate([
            line.widthAnchor.constraint(equalToConstant: 0.5),
            line.heightAnchor.constraint(equalToConstant: 18),
        ])
        return line
    }

    private func makeChip(for item: ChipItem) -> UIButton {
        let btn = UIButton(type: .system)
        // `UIButton.Configuration` is what replaced `contentEdgeInsets`,
        // which iOS 15 deprecated and ignores entirely once a button has
        // a configuration. Title, colour and font move into it for the
        // same reason: a `titleLabel.font` written outside is discarded
        // on the configuration's next update.
        var cfg = UIButton.Configuration.plain()
        cfg.title = item.title
        cfg.baseForegroundColor = .label
        cfg.contentInsets = NSDirectionalEdgeInsets(
            top: 0, leading: 10, bottom: 0, trailing: 10
        )
        // Same family/weight Apple uses on QuickType so the strip
        // looks native and the user reads it without re-tuning.
        cfg.titleTextAttributesTransformer =
            UIConfigurationTextAttributesTransformer { attrs in
                var out = attrs
                out.font = .systemFont(ofSize: 15, weight: .regular)
                return out
            }
        btn.configuration = cfg
        btn.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            btn.heightAnchor.constraint(equalToConstant: 30),
        ])
        btn.addAction(
            UIAction { [weak self] _ in self?.chipTapped(item) },
            for: .touchUpInside
        )
        return btn
    }

    private func chipTapped(_ item: ChipItem) {
        guard let web = webView else { return }
        let slug = JSStringEscape.singleQuoted(item.slug)
        let kind = JSStringEscape.singleQuoted(item.kind.rawValue)
        let js = "window.__outlSuggesterPicked && window.__outlSuggesterPicked('\(slug)', '\(kind)')"
        web.evaluateJavaScript(js, completionHandler: nil)
    }
}
