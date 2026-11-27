import OutlKit
import ObjectiveC.runtime
import UIKit
import WebKit

/// Replaces the `inputAccessoryView` of WebKit's private `WKContentView`
/// with our `OutlToolbarView` via method swizzling.
///
/// **Why we have to swizzle:** the iOS form input accessory bar
/// (`↑ ↓ ✓`) is supplied by `WKContentView`'s default
/// `inputAccessoryView` getter, and there's no public API to swap it.
/// `WKContentView` is a private class in WebKit, so we look it up by
/// name and replace the method's IMP at runtime with a closure that
/// returns our toolbar singleton.
///
/// **Why this lives in Swift now:** the original Obj-C version called
/// `imp_implementationWithBlock` directly. Swift can do the same via
/// `ObjectiveC.runtime` — the only twist is the closure must be
/// `@convention(block)` so its ABI matches what `imp_implementationWithBlock`
/// expects.
@objc(OutlSwizzle)
public final class OutlSwizzle: NSObject {

    /// Toolbar singleton handed back to UIKit as the keyboard's
    /// `inputAccessoryView`. One per app lifetime so the same view
    /// instance animates with the keyboard across every focus change.
    private static var sharedToolbar: OutlToolbarView?

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    /// Schedules all the steps that have to run after `main()`
    /// finishes wiring UIKit.
    @objc public static func install() {
        DispatchQueue.main.async {
            installSwizzle()
        }
    }

    // MARK: - Swizzle install

    private static var swizzleRetries = 0

    private static func installSwizzle() {
        // `WKContentView` isn't registered with the Obj-C runtime
        // until at least one `WKWebView` exists. Retry up to ~1s
        // after launch so we catch it without busy-looping.
        guard let cls: AnyClass = NSClassFromString("WKContentView") else {
            if swizzleRetries >= 10 {
                NSLog("[outl] gave up looking for WKContentView")
                return
            }
            swizzleRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                installSwizzle()
            }
            return
        }

        let sel = #selector(getter: UIResponder.inputAccessoryView)
        guard let method = class_getInstanceMethod(cls, sel) else { return }

        // The block here MUST be `@convention(block)`. Without it, the
        // Swift closure ABI doesn't match what `imp_implementationWithBlock`
        // builds — you'd get a crash the first time UIKit dispatches
        // to the swizzled IMP.
        let block: @convention(block) (AnyObject) -> UIView? = { _ in
            if sharedToolbar == nil {
                sharedToolbar = OutlToolbarView()
            }
            return sharedToolbar
        }
        let imp = imp_implementationWithBlock(block)
        method_setImplementation(method, imp)
        NSLog("[outl] installed native toolbar (with embedded suggester)")

        neutralizeSmartPunctuation(on: cls)
        disableInteractiveDismiss()
        bindWebView()
    }

    // MARK: - Neutralize iOS Smart Punctuation

    /// iOS "Smart Punctuation" rewrites `--` → `–`, `...` → `…`, and
    /// straight quotes → curly *after* the user types — corrupting a
    /// markdown outliner's code and CLI snippets. The HTML editor used
    /// to suppress it with `autocorrect="off"`, but that also hid the
    /// QuickType prediction bar the user types with.
    ///
    /// The precise fix: leave autocorrect/QuickType on in the HTML
    /// (`BlockRow.tsx` no longer sets `autocorrect`/`spellcheck`) and
    /// force just the three smart-punctuation traits to `.no` on the
    /// private `WKContentView`. These are `UITextInputTraits` getters;
    /// UIKit reads them off the first responder every time it resolves
    /// the keyboard's behaviour, so replacing each IMP with a constant
    /// `.no` is enough — no per-focus re-application needed.
    private static func neutralizeSmartPunctuation(on cls: AnyClass) {
        forceTrait(cls, "smartQuotesType", UITextSmartQuotesType.no.rawValue)
        forceTrait(cls, "smartDashesType", UITextSmartDashesType.no.rawValue)
        forceTrait(cls, "smartInsertDeleteType", UITextSmartInsertDeleteType.no.rawValue)
        NSLog("[outl] neutralized smart punctuation (kept QuickType)")
    }

    /// Replace a `UITextInputTraits` enum getter's IMP with one that
    /// returns a constant value. Best-effort: if the getter isn't found
    /// (WebKit internals change across iOS versions) we log and move on
    /// rather than crash — the worst case is smart punctuation stays on,
    /// not a broken keyboard.
    private static func forceTrait(_ cls: AnyClass, _ selectorName: String, _ value: Int) {
        let sel = NSSelectorFromString(selectorName)
        guard let method = class_getInstanceMethod(cls, sel) else {
            NSLog("[outl] no \(selectorName) on WKContentView — smart punctuation left as-is")
            return
        }
        // Same `@convention(block)` ABI requirement as the
        // inputAccessoryView swizzle above; the getter returns an
        // `NSInteger`-backed enum, so a constant `Int` block is a valid
        // IMP.
        let block: @convention(block) (AnyObject) -> Int = { _ in value }
        method_setImplementation(method, imp_implementationWithBlock(block))
    }

    // MARK: - Disable interactive keyboard dismiss

    private static var dismissRetries = 0

    /// iOS's "interactive keyboard dismiss" gesture lets the user
    /// drag down on a `UIScrollView` to slide the keyboard out. Our
    /// outline is itself a scroll view, so without this fix any
    /// down-drag inside the editor partially dismisses the keyboard
    /// and drags the toolbar along — confusing UX.
    private static func disableInteractiveDismiss() {
        guard let win = OutlWindows.key,
              let web = OutlToolbarView.findWebView(in: win)
        else {
            if dismissRetries >= 10 { return }
            dismissRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                disableInteractiveDismiss()
            }
            return
        }
        web.scrollView.keyboardDismissMode = .none
        NSLog("[outl] disabled interactive keyboard dismiss")
    }

    // MARK: - Bind WebView into toolbar + suggester overlay

    private static var bindRetries = 0

    private static func bindWebView() {
        guard let win = OutlWindows.key,
              let web = OutlToolbarView.findWebView(in: win)
        else {
            if bindRetries >= 20 { return }
            bindRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                bindWebView()
            }
            return
        }
        if sharedToolbar == nil {
            sharedToolbar = OutlToolbarView()
        }
        sharedToolbar?.webView = web
        OutlSuggestOverlay.install(webView: web)
        NSLog("[outl] bound suggester webview (overlay-driven)")
    }

}

/// Where the app's key window comes from, for every file in this target.
///
/// Six call sites asked `UIApplication.shared.windows.first`, deprecated
/// since iOS 15 and wrong on any multi-scene layout: it answers with a
/// window from whichever scene the array happens to start with. Two of
/// them had grown their own scene walk on top of it, which is how one
/// lookup ends up with three spellings and only some of them current.
enum OutlWindows {

    /// The foreground scene's key window, else any window at all.
    ///
    /// The second pass is not a formality: a Darwin notification from an
    /// extension and a background refresh both run with no scene
    /// `foregroundActive`, and the deprecated call used to find a window
    /// for them. Dropping it would make those paths silently no-op.
    static var key: UIWindow? {
        let scenes = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
        for scene in scenes where scene.activationState == .foregroundActive {
            if let w = scene.keyWindow ?? scene.windows.first { return w }
        }
        for scene in scenes {
            if let w = scene.keyWindow ?? scene.windows.first { return w }
        }
        return nil
    }
}
