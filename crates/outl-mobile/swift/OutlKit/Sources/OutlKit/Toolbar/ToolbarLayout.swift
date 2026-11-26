import Foundation

/// The geometry the floating keyboard toolbar owns, and the scripts that
/// tell the page how much of its bottom edge the toolbar covers.
///
/// The toolbar is a UIKit overlay in the key window, pinned above the
/// keyboard. It is not part of the keyboard any more, so WebKit's
/// `interactive-widget=resizes-content` shrinks the viewport to the keys
/// alone and the pill sits on top of the page's last rows. The page needs
/// to know by how much, and this file is the one place that number lives:
/// the UIKit view sizes itself from `stackHeight`, and the clearance it
/// publishes is the same number, so the two cannot drift.
///
/// Kept free of UIKit so `swift test` can pin the numbers and the script
/// text without a device.
public enum ToolbarLayout {

    /// Height of the capsule itself.
    public static let capsuleHeight: Double = 44

    /// Space above and below the capsule inside the overlay. The one
    /// below is the gap to the keys; the one above is the gap the
    /// suggester strip keeps from the pill.
    public static let verticalMargin: Double = 6

    /// The whole overlay: capsule plus its margins. This is also what
    /// the page must keep clear while the pill is up.
    public static let stackHeight: Double = capsuleHeight + 2 * verticalMargin

    /// The CSS custom property the page reads (`styles.css`). Declared
    /// there as `0px`, so a page with no toolbar, or a toolbar that is
    /// hidden, resolves to no clearance.
    public static let clearanceProperty = "--outl-toolbar-clearance"

    /// Script that publishes the clearance and then reveals the focused
    /// block above the pill.
    ///
    /// The reveal is `block: nearest`, so a field that is already clear
    /// of the pill does not move. It runs now and once more on the next
    /// visual-viewport resize, because the keyboard's resize may land
    /// before or after this script does.
    public static var showClearanceScript: String {
        """
        (function(){
        document.documentElement.style.setProperty('\(clearanceProperty)','\(Int(stackHeight))px');
        var a=document.activeElement;
        if(!a||a.tagName!=='TEXTAREA'||!a.scrollIntoView)return;
        function reveal(){if(a.isConnected&&a===document.activeElement)a.scrollIntoView({block:'nearest'});}
        reveal();
        if(window.visualViewport)window.visualViewport.addEventListener('resize',reveal,{once:true});
        })()
        """
    }

    /// Script that drops the clearance, back to the stylesheet's `0px`.
    public static var hideClearanceScript: String {
        "document.documentElement.style.removeProperty('\(clearanceProperty)')"
    }

    /// Expression answering "is the caret in the outline editor?", as
    /// opposed to a field in a sheet. The editor is the only `<textarea>`
    /// outside a `role="dialog"` (the off-screen keyboard keeper counts:
    /// it only ever holds focus for a block create).
    public static let editorFocusedScript =
        "(function(){var a=document.activeElement;"
        + "return !!(a&&a.tagName==='TEXTAREA'&&!a.closest('[role=\"dialog\"]'));})()"

    /// Reads what `evaluateJavaScript` returned for `editorFocusedScript`.
    ///
    /// An answer that did not come back (JS not ready, an error) is
    /// "yes": the old accessory showed for every field, and a toolbar
    /// that disappears because a read failed is worse than one that shows
    /// over a sheet.
    public static func isEditorFocused(_ result: Any?) -> Bool {
        (result as? Bool) ?? true
    }
}
