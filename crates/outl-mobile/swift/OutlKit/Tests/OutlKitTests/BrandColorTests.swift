import XCTest
@testable import OutlKit

final class BrandColorTests: XCTestCase {

    func testDarkIsHex0c0814() {
        // This is the canonical outl deep purple. It also lives in:
        //   • LaunchScreen.storyboard background
        //   • crates/outl-mobile/src/styles.css (--color-iosd-bg)
        //   • the marketing site palette
        // If they drift, cold launch flashes white-on-purple. We
        // freeze the value here so a future refactor doesn't change
        // it without knowing what else it touches.
        let c = BrandColor.dark
        XCTAssertEqual(c.red, 0x0c)
        XCTAssertEqual(c.green, 0x08)
        XCTAssertEqual(c.blue, 0x14)
        XCTAssertEqual(c.hex, "0c0814")
    }

    func testIconGradientMatchesTheGeneratedTiles() {
        // The widget paints the same two-stop gradient every app icon
        // is rasterised from, so a tile on the Home Screen and the icon
        // beside it are the same purple. The source of those tiles is
        // scripts/gen-icons.py (VIOLET, LAVENDER, PAPER) and
        // assets/brand/*.svg. There is no build step tying the Swift
        // side to the Python one, so this test is the tie.
        XCTAssertEqual(BrandColor.violet.hex, "7c3aed")
        XCTAssertEqual(BrandColor.lavender.hex, "a78bfa")
        XCTAssertEqual(BrandColor.paper.hex, "f4f1fa")
    }

    func testComponentsAreUnitScale() {
        let comps = BrandColor.dark.components
        XCTAssertEqual(comps.red, 12.0 / 255.0, accuracy: 1e-9)
        XCTAssertEqual(comps.green, 8.0 / 255.0, accuracy: 1e-9)
        XCTAssertEqual(comps.blue, 20.0 / 255.0, accuracy: 1e-9)
    }

    func testEquatable() {
        XCTAssertEqual(
            BrandColor(red: 12, green: 8, blue: 20),
            BrandColor.dark
        )
        XCTAssertNotEqual(
            BrandColor(red: 12, green: 8, blue: 21),
            BrandColor.dark
        )
    }
}
