import XCTest
@testable import OutlKit

final class FlavorTests: XCTestCase {
    /// What `swift test` itself is: no app bundle, so no keys.
    func testNoKeysResolvesToProduction() {
        XCTAssertEqual(OutlFlavor.resolve { _ in nil }, .production)
        XCTAssertTrue(OutlFlavor.resolve { _ in nil }.isProduction)
    }

    func testPreviewKeysAreRead() {
        let f = OutlFlavor.resolve { key in
            [
                "OutlAppGroup": "group.app.outl.mobile-app.preview",
                "OutlURLScheme": "outl-preview",
                "OutlCaptureNotification": "app.outl.capture.arrived.preview",
            ][key]
        }
        XCTAssertEqual(f.appGroup, "group.app.outl.mobile-app.preview")
        XCTAssertEqual(f.urlScheme, "outl-preview")
        XCTAssertEqual(f.captureNotification, "app.outl.capture.arrived.preview")
        XCTAssertFalse(f.isProduction)
    }

    /// A build setting that is not defined reaches the plist verbatim.
    /// `$(OUTL_APP_GROUP)` is not an App Group, so it is not a value.
    func testUnsubstitutedSettingIsNotAValue() {
        let f = OutlFlavor.resolve { _ in "$(OUTL_APP_GROUP)" }
        XCTAssertEqual(f, .production)
    }

    func testBlankIsNotAValue() {
        XCTAssertEqual(OutlFlavor.resolve { _ in "   " }, .production)
        XCTAssertEqual(OutlFlavor.resolve { _ in "" }, .production)
    }

    /// Per key, so a bundle missing one still gets the others. A
    /// wholesale fallback would hand a preview build the production App
    /// Group over a single missing scheme.
    func testResolutionIsPerKey() {
        let f = OutlFlavor.resolve { key in
            key == "OutlAppGroup" ? "group.app.outl.mobile-app.preview" : nil
        }
        XCTAssertEqual(f.appGroup, "group.app.outl.mobile-app.preview")
        XCTAssertEqual(f.urlScheme, OutlFlavor.production.urlScheme)
    }

    /// The three names a preview build must not share with the real app.
    /// Not a tautology: it fails if a later change derives one of them
    /// from a literal instead of from the flavor.
    func testPreviewSharesNoNameWithProduction() {
        let preview = OutlFlavor(
            appGroup: "group.app.outl.mobile-app.preview",
            urlScheme: "outl-preview",
            captureNotification: "app.outl.capture.arrived.preview"
        )
        let p = OutlFlavor.production
        XCTAssertNotEqual(preview.appGroup, p.appGroup)
        XCTAssertNotEqual(preview.urlScheme, p.urlScheme)
        XCTAssertNotEqual(preview.captureNotification, p.captureNotification)
    }
}
