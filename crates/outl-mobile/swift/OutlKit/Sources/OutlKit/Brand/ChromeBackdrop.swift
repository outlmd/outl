import Foundation

/// The decisions behind the fill the iOS keyboard toolbar sits on.
///
/// The accessory view floats a white capsule over a strip that used to
/// be transparent, so whatever iOS hosts the accessory in showed through
/// (solid black in light mode). The UIKit side fills the strip, and
/// repaints the host views around it, with `lightWhite` / `darkWhite`;
/// the suggester strip above it reads the same two numbers, which is
/// what keeps the two surfaces one continuous slab.
///
/// Kept free of UIKit so `swift test` can pin the numbers and the
/// "is this host safe to repaint" rule without a device.
public enum ChromeBackdrop {

    /// Grey level of the strip in light mode. Quiet next to the light
    /// keyboard, and darker than the white capsule so its shadow reads.
    public static let lightWhite: Double = 0.90

    /// Grey level of the strip in dark mode. Lighter than the near-black
    /// hosts it replaces, darker than the dark capsule (`0.18`).
    public static let darkWhite: Double = 0.11

    /// Highest channel value still counted as "black" for a host fill.
    public static let nearBlackCeiling: Double = 0.10

    /// Alpha at or below which a fill is treated as no fill at all.
    public static let clearCeiling: Double = 0.05

    /// Tallest host view we will repaint, in points. The accessory is
    /// 56 pt and the tallest keyboard (iPad, with its shortcut bar) is
    /// well under this; a window-sized view is not ours to fill.
    public static let hostHeightLimit: Double = 480

    /// Whether a host's current fill is one we may replace: absent,
    /// transparent, or near black. Anything else (a colour, a grey the
    /// keyboard chose) is left alone.
    public static func isReplaceableFill(
        red: Double, green: Double, blue: Double, alpha: Double
    ) -> Bool {
        if alpha <= clearCeiling { return true }
        return max(red, green, blue) <= nearBlackCeiling
    }

    /// Whether a host of this height may be repainted at all.
    /// A view as tall as the screen is the window itself or the app
    /// behind it, and an opaque fill there would hide the page.
    public static func isKeyboardSized(hostHeight: Double, screenHeight: Double) -> Bool {
        hostHeight > 0 && hostHeight <= hostHeightLimit && hostHeight < screenHeight
    }
}
