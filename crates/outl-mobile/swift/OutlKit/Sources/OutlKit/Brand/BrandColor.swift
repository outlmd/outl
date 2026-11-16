import Foundation

/// Outl's deep-purple brand color, expressed as an RGB triplet (0–255).
///
/// Lives here rather than inline in `OutlBrandChrome` so a single
/// `swift test` run can guarantee the value never drifts away from
/// `#0c0814` — the same color baked into `LaunchScreen.storyboard`,
/// the marketing site, and the Tailwind palette (`--color-iosd-bg`).
/// If those drift, the cold launch flashes white-on-purple, which is
/// exactly the bug `OutlBrandChrome` exists to suppress.
public struct BrandColor: Equatable, Sendable {
    public let red: UInt8
    public let green: UInt8
    public let blue: UInt8

    public init(red: UInt8, green: UInt8, blue: UInt8) {
        self.red = red
        self.green = green
        self.blue = blue
    }

    /// `#0c0814` — the canonical outl dark.
    public static let dark = BrandColor(red: 12, green: 8, blue: 20)

    /// `#7c3aed` — the deep end of the app icon's gradient.
    public static let violet = BrandColor(red: 124, green: 58, blue: 237)

    /// `#a78bfa` — the light end of the app icon's gradient.
    public static let lavender = BrandColor(red: 167, green: 139, blue: 250)

    /// `#f4f1fa` — the mark's own off-white, on every tile.
    public static let paper = BrandColor(red: 244, green: 241, blue: 250)

    /// Components in the 0…1 range UIKit / SwiftUI / CG expect.
    public var components: (red: Double, green: Double, blue: Double) {
        (Double(red) / 255.0, Double(green) / 255.0, Double(blue) / 255.0)
    }

    /// Hex string, lowercase, no leading `#`. Used by tests and
    /// debug log lines.
    public var hex: String {
        String(format: "%02x%02x%02x", red, green, blue)
    }
}
