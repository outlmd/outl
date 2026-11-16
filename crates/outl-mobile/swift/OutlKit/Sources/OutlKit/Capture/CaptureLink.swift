import Foundation

/// The deep links the iOS extensions open. The app routes them through
/// `outl_actions::deeplink` (`DeepLinkTarget::Capture` opens the capture
/// sheet), so the widget never needs to know how capture works.
public enum CaptureLink {
    /// `outl://capture`: open the app on the capture sheet for today.
    ///
    /// The scheme is the flavor's (`OutlFlavor`), so a preview build's
    /// widget opens the preview app rather than whichever of the two iOS
    /// decided owns `outl://` today.
    public static let capture =
        URL(string: "\(OutlFlavor.current.urlScheme)://capture")
        ?? URL(string: "\(OutlFlavor.production.urlScheme)://capture")!

    /// The label every capture entry point shows. One string so the
    /// widget, the intent and the share sheet read the same.
    public static let title = "Add to today"
}
