import XCTest
@testable import OutlKit

/// `ToolbarStore` parses what one `evaluateJavaScript` read of the
/// webview's `localStorage` returned. Every case here is a payload the
/// bar can actually receive on a device.
final class ToolbarStoreTests: XCTestCase {

    private func pair(_ mfu: String?, _ lock: String?) -> String {
        let slot = { (v: String?) -> String in
            guard let v else { return "null" }
            let escaped = v
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            return "\"\(escaped)\""
        }
        return "[\(slot(mfu)),\(slot(lock))]"
    }

    // MARK: - parsePair

    func testReadsCountsAndLockTogether() {
        let raw = pair("{\"bold\":3}", "[\"code\"]")
        let result = ToolbarStore.parsePair(raw)
        XCTAssertEqual(result?.counts["bold"], 3)
        XCTAssertEqual(result?.locked?.first, .code)
    }

    /// A device that never tapped a button and never locked the bar.
    /// Both slots absent is the normal first-launch state, not an
    /// error.
    func testBothSlotsMissingIsAValidEmptyRead() {
        let result = ToolbarStore.parsePair(pair(nil, nil))
        XCTAssertEqual(result?.counts, [:])
        XCTAssertNil(result?.locked)
    }

    func testCountsWithoutLockIsUnlocked() {
        let result = ToolbarStore.parsePair(pair("{\"bold\":1}", nil))
        XCTAssertEqual(result?.counts["bold"], 1)
        XCTAssertNil(result?.locked)
    }

    /// An unusable payload must be distinguishable from an empty one:
    /// the caller keeps its previous values on `nil`, and clearing on
    /// a failed read is exactly how a locked bar would silently
    /// unlock.
    func testUnusablePayloadReturnsNilRatherThanEmpty() {
        XCTAssertNil(ToolbarStore.parsePair(nil))
        XCTAssertNil(ToolbarStore.parsePair(""))
        XCTAssertNil(ToolbarStore.parsePair("not json"))
        XCTAssertNil(ToolbarStore.parsePair("{\"counts\":{}}"))
        XCTAssertNil(ToolbarStore.parsePair("[]"))
        XCTAssertNil(ToolbarStore.parsePair("[null]"))
        XCTAssertNil(ToolbarStore.parsePair("[null,null,null]"))
    }

    // MARK: - parseCounts (mirrors the TS `parseCounts`)

    func testCountsTolerateMalformedInput() {
        XCTAssertEqual(ToolbarStore.parseCounts(nil), [:])
        XCTAssertEqual(ToolbarStore.parseCounts("not json"), [:])
        XCTAssertEqual(ToolbarStore.parseCounts("[1,2,3]"), [:])
        XCTAssertEqual(ToolbarStore.parseCounts("42"), [:])
    }

    func testCountsDropUnknownIdsAndNonNumbers() {
        let raw = "{\"code\":3,\"bogus\":5,\"italic\":\"x\",\"bold\":2.9}"
        let counts = ToolbarStore.parseCounts(raw)
        XCTAssertEqual(counts, ["code": 3, "bold": 2])
    }

    /// JSON has one number type, so `true` bridges to `NSNumber` too.
    /// A boolean is not a count.
    func testCountsDropBooleans() {
        XCTAssertEqual(ToolbarStore.parseCounts("{\"bold\":true}"), [:])
        XCTAssertEqual(ToolbarStore.parseCounts("{\"bold\":false}"), [:])
    }

    /// Regression: an `NSNumber` holding 1 bridges to `true`, so a
    /// `!(value is Bool)` guard silently dropped every count of 1 —
    /// which is every button's **first** tap. The bar would then never
    /// promote a button until its second use.
    func testCountOfOneSurvives() {
        XCTAssertEqual(ToolbarStore.parseCounts("{\"bold\":1}"), ["bold": 1])
        XCTAssertEqual(ToolbarStore.parseCounts("{\"bold\":0}"), ["bold": 0])
    }

    /// `localStorage` is untrusted: a number outside `Int`'s range would
    /// trap in `Int(Double)` and take the whole bar down. It is dropped
    /// instead, and the counts around it survive.
    func testCountsOutsideIntRangeAreDroppedNotTrapped() {
        let raw = "{\"bold\":1e300,\"code\":-1e300,\"italic\":2}"
        XCTAssertEqual(ToolbarStore.parseCounts(raw), ["italic": 2])
    }

    // MARK: - cross-language contract

    /// The bar interpolates both keys into the JS it evaluates, and
    /// `@outl/shared/toolbar` writes them from TypeScript. Nothing
    /// fails at build time if one side is renamed — the bar just stops
    /// seeing the user's taps and lock, silently. The TS side pins the
    /// same two literals in `keys.test.ts`.
    func testStorageKeysMatchTheTypeScriptContract() {
        XCTAssertEqual(ToolbarMFU.storageKey, "outl.toolbar.mfu.v1")
        XCTAssertEqual(ToolbarLock.storageKey, "outl.toolbar.lock.v1")
    }
}
