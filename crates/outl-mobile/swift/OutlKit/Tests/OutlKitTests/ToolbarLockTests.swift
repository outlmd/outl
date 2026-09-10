import XCTest
@testable import OutlKit

/// `ToolbarLock` keeps no store of its own — the lock lives in the
/// webview's `localStorage` and `OutlToolbarView` reads it from there —
/// so every one of these exercises a pure function over an explicit
/// input. No `UserDefaults` suite to isolate.
final class ToolbarLockTests: XCTestCase {

    /// The set a frozen order has to account for.
    private var middle: [ToolbarAction] { ToolbarAction.middleOrder }

    // MARK: - reconcile

    func testKeepsAFullSnapshotExactlyAsSaved() {
        let frozen = middle.reversed().map(\.rawValue)
        XCTAssertEqual(
            ToolbarLock.reconcile(frozen).map(\.rawValue),
            frozen
        )
    }

    func testAppendsActionsTheSnapshotNeverSaw() {
        // A lock taken before `bold` / `italic` shipped.
        let old = middle.filter { $0 != .bold && $0 != .italic }
        let order = ToolbarLock.reconcile(old.map(\.rawValue))
        XCTAssertEqual(order.count, middle.count)
        XCTAssertEqual(Array(order.prefix(old.count)), old)
        XCTAssertEqual(Array(order.suffix(2)), [.bold, .italic])
    }

    func testDropsIdsTheCatalogNoLongerHas() {
        let order = ToolbarLock.reconcile(["bold", "sendCarrierPigeon"])
        XCTAssertEqual(order.first, .bold)
        XCTAssertEqual(order.count, middle.count)
    }

    func testDropsThePinnedSlots() {
        let order = ToolbarLock.reconcile([
            ToolbarAction.pinnedFirst.rawValue,
            "bold",
            ToolbarAction.pinnedLast.rawValue,
        ])
        XCTAssertFalse(order.contains(ToolbarAction.pinnedFirst))
        XCTAssertFalse(order.contains(ToolbarAction.pinnedLast))
        XCTAssertEqual(order.first, .bold)
    }

    func testDeDupesKeepingFirstAppearance() {
        let order = ToolbarLock.reconcile(["bold", "italic", "bold"])
        XCTAssertEqual(Array(order.prefix(2)), [.bold, .italic])
        XCTAssertEqual(Set(order).count, middle.count)
    }

    func testAlwaysReturnsTheWholeMiddleRange() {
        for saved in [[], ["nonsense"], ["bold"], middle.map(\.rawValue)] {
            let order = ToolbarLock.reconcile(saved)
            XCTAssertEqual(Set(order), Set(middle))
            XCTAssertEqual(order.count, middle.count)
        }
    }

    // MARK: - parse

    func testParseReadsMissingAndMalformedAsNotLocked() {
        XCTAssertNil(ToolbarLock.parse(nil))
        XCTAssertNil(ToolbarLock.parse(""))
        XCTAssertNil(ToolbarLock.parse("not json"))
        XCTAssertNil(ToolbarLock.parse("{\"bold\":1}"))
        XCTAssertNil(ToolbarLock.parse("42"))
    }

    func testParseReadsAnEmptyArrayAsLockedOnTheCatalogOrder() {
        XCTAssertEqual(ToolbarLock.parse("[]"), middle)
    }

    func testParseIgnoresNonStringEntries() {
        let order = ToolbarLock.parse("[\"bold\", 7, null, \"italic\"]")
        XCTAssertEqual(Array(order?.prefix(2) ?? []), [.bold, .italic])
    }

    // MARK: - resolve

    func testResolveFollowsMFUWhenUnlocked() {
        let counts = ["code": 10]
        XCTAssertEqual(
            ToolbarLock.resolveMiddleActions(counts: counts, locked: nil),
            ToolbarMFU.orderedMiddleActions(counts: counts)
        )
    }

    func testResolveIgnoresCountsWhenLocked() {
        let frozen = Array(middle.reversed())
        XCTAssertEqual(
            ToolbarLock.resolveMiddleActions(counts: ["code": 9999], locked: frozen),
            frozen
        )
    }

    // MARK: - cross-language contract

    /// The key is read by `OutlToolbar.swift` out of the webview, and
    /// written by `@outl/shared/toolbar`'s `LOCK_STORAGE_KEY`. Nothing
    /// fails at build time if one side is renamed: the bar simply
    /// stops seeing the lock the user set, silently. Pin the string.
    func testStorageKeyMatchesTheTypeScriptContract() {
        XCTAssertEqual(ToolbarLock.storageKey, "outl.toolbar.lock.v1")
    }

    /// Same argument for the MFU key, whose Swift and TS sides are two
    /// independent stores that must at least agree on the name.
    func testMFUStorageKeyMatchesTheTypeScriptContract() {
        XCTAssertEqual(ToolbarMFU.storageKey, "outl.toolbar.mfu.v1")
    }
}
