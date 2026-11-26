import OutlKit
import UIKit
import WebKit

/// Paints the UIWindow + WKWebView in `BrandColor.dark` (`#0c0814`)
/// as early as possible during launch, then hands them over to the
/// theme-aware page background once the page has loaded.
///
/// **Why this exists:** iOS hands off from `LaunchScreen.storyboard`
/// (brand-dark) to the Tauri WebView. `WKWebView` defaults to
/// `opaque = true` + white background — that white flashed before the
/// first HTML frame rendered, breaking the visual continuity from
/// LaunchScreen → app.
///
/// We fix it by walking the window tree as early as possible:
///   - tint the UIWindow itself, covering any gap between view
///     transitions where the window is briefly visible
///   - mark the WKWebView non-opaque with a brand-coloured background
///     so even its very first frame matches LaunchScreen
///   - apply the same colour to `WKWebView.scrollView` so
///     bounce / overscroll doesn't reveal a white seam
///
/// **The brand fill is a launch fill, not an in-session backdrop.**
/// Left in place it is what shows in light mode wherever the page does
/// not cover the window (overscroll bounce, a window sliver): an
/// almost-black strip under a white page. So once the
/// document has loaded, `settle` swaps all four surfaces to
/// `OutlChrome.pageBackground`, a dynamic colour (white in light,
/// `BrandColor.dark` in dark) that UIKit re-resolves on a light/dark
/// flip. The swap waits for the load because the webview is
/// non-opaque until then: a light window under a dark LaunchScreen
/// would bring the white flash back. `index.html` already draws its
/// own splash in the matching colour from the first frame, so the swap
/// is invisible.
///
/// The 0…1-second window between launch and "WebView mounted" is
/// covered by a retry loop with a hard cap so we never spin forever
/// if something genuinely broke. Brand source-of-truth lives in
/// `OutlKit.BrandColor.dark`; freezing that value is what the unit
/// test in `BrandColorTests.testDarkIsHex0c0814` guards.
@objc(OutlBrandChrome)
public final class OutlBrandChrome: NSObject {

    private static var windowRetries = 0
    private static var webRetries = 0
    private static var loadObservation: NSKeyValueObservation?
    private static var traitRegistration: (any UITraitChangeRegistration)?
    private static var settled = false

    /// How long after the webview appears we settle regardless of its
    /// load state. Same budget as `index.html`'s splash failsafe: a
    /// page that never loads must not keep the launch fill forever.
    private static let settleFailsafe: TimeInterval = 4

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        DispatchQueue.main.async {
            apply()
        }
    }

    private static func apply() {
        guard let win = OutlWindows.key else {
            if windowRetries >= 20 {
                return
            }
            windowRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                apply()
            }
            return
        }

        let brand = uiColor(from: BrandColor.dark)
        win.backgroundColor = brand
        win.rootViewController?.view.backgroundColor = brand

        if let web = OutlToolbarView.findWebView(in: win) {
            web.isOpaque = false
            web.backgroundColor = brand
            web.scrollView.backgroundColor = brand
            NSLog("[outl] brand chrome applied (window + webview)")
            settleWhenLoaded(web)
            return
        }

        // WebView not mounted yet — keep polling so we catch its
        // first frame. The window is already tinted so the user
        // sees brand colour throughout this window.
        if webRetries >= 30 {
            NSLog("[outl] brand chrome: webview never mounted, window-only")
            return
        }
        webRetries += 1
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            apply()
        }
    }

    // MARK: - Hand-over to the page background

    /// Wait for the document to finish loading, then `settle`. A load
    /// that already finished (the observer attached late) settles at
    /// once; one that never does is cut off by the failsafe.
    private static func settleWhenLoaded(_ web: WKWebView) {
        if hasLoaded(web) {
            settle()
            return
        }
        loadObservation = web.observe(\.isLoading, options: [.new]) { web, _ in
            if hasLoaded(web) {
                DispatchQueue.main.async { settle() }
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + settleFailsafe) {
            settle()
        }
    }

    /// `url` distinguishes "finished" from "not started yet": before
    /// the first navigation `isLoading` is also false.
    private static func hasLoaded(_ web: WKWebView) -> Bool {
        web.url != nil && !web.isLoading
    }

    /// Replace the launch fill with `OutlChrome.pageBackground` on the
    /// window, the root view, the webview and its scroll view, and keep
    /// it fresh across a light/dark flip. Idempotent: the observer and
    /// the failsafe both call it.
    private static func settle() {
        guard let win = OutlWindows.key else { return }
        paintPage(in: win)
        guard !settled else { return }
        settled = true
        loadObservation = nil
        // The colour is dynamic, so UIKit re-resolves it on its own for
        // a view's `backgroundColor`; the registration covers the
        // surfaces that cache a resolved value (`underPageBackgroundColor`).
        traitRegistration = win.registerForTraitChanges(
            [UITraitUserInterfaceStyle.self]
        ) { (window: UIWindow, _) in
            paintPage(in: window)
        }
        NSLog("[outl] brand chrome settled on the page background")
    }

    private static func paintPage(in win: UIWindow) {
        let page = OutlChrome.pageBackground
        win.backgroundColor = page
        win.rootViewController?.view.backgroundColor = page
        if let web = OutlToolbarView.findWebView(in: win) {
            web.backgroundColor = page
            web.scrollView.backgroundColor = page
            web.underPageBackgroundColor = page
        }
    }

    /// `BrandColor.dark` as UIKit sees it.
    static func brandUIColor() -> UIColor {
        uiColor(from: BrandColor.dark)
    }

    private static func uiColor(from c: BrandColor) -> UIColor {
        let comps = c.components
        return UIColor(
            red: CGFloat(comps.red),
            green: CGFloat(comps.green),
            blue: CGFloat(comps.blue),
            alpha: 1.0
        )
    }
}


/// The surface colour the native chrome shares, so a client file never
/// spells its own.
///
/// Dynamic, resolved against the trait collection of whatever draws it,
/// which is how the capsule already follows light/dark.
enum OutlChrome {

    /// The page behind the outline: white in light (`outl` theme `bg`),
    /// `BrandColor.dark` in dark. Dark is deliberately the launch fill,
    /// so a dark launch never changes colour at the hand-over.
    static let pageBackground = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? OutlBrandChrome.brandUIColor()
            : UIColor(white: 1.0, alpha: 1.0)
    }
}
