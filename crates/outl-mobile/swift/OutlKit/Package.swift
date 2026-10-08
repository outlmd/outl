// swift-tools-version:5.7
//
// `OutlKit` hosts every piece of `outl-mobile`'s iOS-side logic that
// can be expressed as pure Foundation — toolbar action ordering,
// suggester chip parsing, iCloud ops file predicate, JS string escape,
// brand color. The iOS app target (`outl-mobile_iOS`) consumes it as a
// local Swift Package dependency, and the test suite runs standalone
// via `swift test` (no UIKit, no device required).
//
// The UIKit / WebKit / BackgroundTasks classes (OutlToolbarView,
// OutlSuggestView, OutlSuggestOverlay, OutlSwizzle, OutlBackgroundRefresh,
// OutlOpsWatcher, OutlBrandChrome) live in the app target itself under
// `src-tauri/gen/apple/Sources/outl-mobile/` and `import OutlKit` for
// the testable pieces.

import PackageDescription

let package = Package(
    name: "OutlKit",
    platforms: [
        .iOS(.v15),
        .macOS(.v11),
    ],
    products: [
        .library(name: "OutlKit", targets: ["OutlKit"]),
    ],
    targets: [
        .target(name: "OutlKit"),
        .testTarget(name: "OutlKitTests", dependencies: ["OutlKit"]),
    ]
)
