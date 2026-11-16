import Foundation

/// Which install of outl this code is running inside.
///
/// A preview build sits on the same phone as the real app, so every name
/// the two could collide on is resolved here rather than written as a
/// literal at each use. Three of them are not scoped by the App Group and
/// would otherwise reach across the two installs:
///
/// - the **App Group** itself, which is the capture inbox's container;
/// - the **URL scheme**, because iOS picks one app for `outl://` in
///   whatever order it likes, so a shared scheme means a Home Screen
///   widget opens either app — and the one it opens writes to its own
///   graph;
/// - the **Darwin notification** an extension posts after a capture,
///   which is machine-global: `CFNotificationCenterGetDarwinNotifyCenter`
///   is not per-app and not per-group, so one name means the real app
///   drains on a capture the preview wrote.
///
/// The values come from the build, through `Outl.xcconfig` → the target's
/// `Info.plist`. That file is the one owner (root `CLAUDE.md`, "one owner
/// per fact"); this type only reads it.
public struct OutlFlavor: Equatable {
    public let appGroup: String
    public let urlScheme: String
    public let captureNotification: String

    public init(appGroup: String, urlScheme: String, captureNotification: String) {
        self.appGroup = appGroup
        self.urlScheme = urlScheme
        self.captureNotification = captureNotification
    }

    /// The shipping app's bundle id. Only ever a fallback: a real process
    /// reads `Bundle.main.bundleIdentifier`, which is authoritative and
    /// already carries the flavor.
    public static let productionBundleId = "app.outl.mobile-app"

    /// What a build with no flavor keys is: the app as it shipped before
    /// preview builds existed.
    public static let production = OutlFlavor(
        appGroup: "group.app.outl.mobile-app",
        urlScheme: "outl",
        captureNotification: "app.outl.capture.arrived"
    )

    /// This process's flavor.
    public static let current = resolve(from: Bundle.main)

    public static func resolve(from bundle: Bundle) -> OutlFlavor {
        resolve { bundle.object(forInfoDictionaryKey: $0) as? String }
    }

    /// Resolution is per key, so a bundle missing one still gets the
    /// others rather than falling back wholesale.
    ///
    /// **Falling back to production is not the safety mechanism, and must
    /// not be read as one.** It exists because `swift test` runs outside
    /// an app bundle, where there is no `Info.plist` to read. What
    /// actually stops a preview build from reaching the real app's data is
    /// the **entitlement**: it is generated from the same `OUTL_APP_GROUP`
    /// as the plist key, so a preview whose plist somehow named the
    /// production group would be refused by the sandbox at the container
    /// lookup. The failure mode is a preview that cannot write its own
    /// captures — loud — never one that writes into the real graph.
    static func resolve(_ value: (String) -> String?) -> OutlFlavor {
        func read(_ key: String, _ fallback: String) -> String {
            guard let v = value(key)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !v.isEmpty,
                  // An unsubstituted build setting (`$(OUTL_APP_GROUP)`)
                  // reaches the plist verbatim when the setting is not
                  // defined. It is not a name, so it is not a value.
                  !v.contains("$(")
            else { return fallback }
            return v
        }
        return OutlFlavor(
            appGroup: read("OutlAppGroup", production.appGroup),
            urlScheme: read("OutlURLScheme", production.urlScheme),
            captureNotification: read("OutlCaptureNotification", production.captureNotification)
        )
    }

    /// `true` when this is the shipping app rather than a build installed
    /// beside it. Used for nothing load-bearing — the names above are what
    /// isolate the two — but it is what a log line should say.
    public var isProduction: Bool { self == .production }
}
