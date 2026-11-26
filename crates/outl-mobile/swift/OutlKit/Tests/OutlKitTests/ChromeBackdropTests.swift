import XCTest
@testable import OutlKit

final class ChromeBackdropTests: XCTestCase {

    func testBackdropGreysAreFrozen() {
        // The suggester strip and the toolbar accessory both read these,
        // so a change here moves two surfaces at once.
        XCTAssertEqual(ChromeBackdrop.lightWhite, 0.90)
        XCTAssertEqual(ChromeBackdrop.darkWhite, 0.11)
    }

    func testLightBackdropIsDarkerThanTheWhiteCapsule() {
        // The capsule is white 1.0 in light mode; a strip as bright
        // would leave only the shadow to tell them apart.
        XCTAssertLessThan(ChromeBackdrop.lightWhite, 1.0)
    }

    func testDarkBackdropIsNotNearBlackAndStaysBelowTheCapsule() {
        // Not a black hole, and the dark capsule (white 0.18) still
        // stands out from it.
        XCTAssertGreaterThan(ChromeBackdrop.darkWhite, ChromeBackdrop.nearBlackCeiling)
        XCTAssertLessThan(ChromeBackdrop.darkWhite, 0.18)
    }

    func testClearAndBlackHostsAreReplaceable() {
        XCTAssertTrue(ChromeBackdrop.isReplaceableFill(red: 0, green: 0, blue: 0, alpha: 0))
        XCTAssertTrue(ChromeBackdrop.isReplaceableFill(red: 1, green: 1, blue: 1, alpha: 0.02))
        XCTAssertTrue(ChromeBackdrop.isReplaceableFill(red: 0, green: 0, blue: 0, alpha: 1))
        // The brand dark (#0c0814) must not survive as a host fill.
        XCTAssertTrue(ChromeBackdrop.isReplaceableFill(
            red: 12.0 / 255, green: 8.0 / 255, blue: 20.0 / 255, alpha: 1))
    }

    func testRealColoursAreLeftAlone() {
        XCTAssertFalse(ChromeBackdrop.isReplaceableFill(red: 1, green: 1, blue: 1, alpha: 1))
        XCTAssertFalse(ChromeBackdrop.isReplaceableFill(red: 0.82, green: 0.83, blue: 0.85, alpha: 1))
        // A translucent grey the keyboard picked is not "clear".
        XCTAssertFalse(ChromeBackdrop.isReplaceableFill(red: 0.5, green: 0.5, blue: 0.5, alpha: 0.5))
        // Already our own dark backdrop: nothing to replace.
        XCTAssertFalse(ChromeBackdrop.isReplaceableFill(
            red: ChromeBackdrop.darkWhite, green: ChromeBackdrop.darkWhite,
            blue: ChromeBackdrop.darkWhite, alpha: 1))
    }

    func testOnlyKeyboardSizedHostsMayBeRepainted() {
        // Accessory + portrait keyboard on a 844 pt screen.
        XCTAssertTrue(ChromeBackdrop.isKeyboardSized(hostHeight: 340, screenHeight: 844))
        // Landscape phone: the host is under the screen height.
        XCTAssertTrue(ChromeBackdrop.isKeyboardSized(hostHeight: 250, screenHeight: 390))
        // The window, or anything as tall as the screen, is never ours.
        XCTAssertFalse(ChromeBackdrop.isKeyboardSized(hostHeight: 844, screenHeight: 844))
        XCTAssertFalse(ChromeBackdrop.isKeyboardSized(hostHeight: 390, screenHeight: 390))
        XCTAssertFalse(ChromeBackdrop.isKeyboardSized(hostHeight: 0, screenHeight: 844))
    }
}
