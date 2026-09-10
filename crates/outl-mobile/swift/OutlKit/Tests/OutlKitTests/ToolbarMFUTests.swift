import XCTest
@testable import OutlKit

final class ToolbarMFUTests: XCTestCase {

    // `ToolbarMFU` is pure ordering now — the counts live in the
    // webview's `localStorage` and are parsed by `ToolbarStore`, so
    // there is no `UserDefaults` suite left to isolate.

    // MARK: - Pure ordering

    func testReturnsDefaultOrderWithEmptyCounts() {
        XCTAssertEqual(
            ToolbarMFU.orderedActions(counts: [:]),
            ToolbarAction.defaultOrder
        )
    }

    func testPinnedFirstStaysAtIndexZero() {
        let order = ToolbarMFU.orderedActions(
            counts: ["italic": 9999, "code": 9999]
        )
        XCTAssertEqual(order.first, ToolbarAction.pinnedFirst)
    }

    func testPinnedLastStaysAtFinalIndex() {
        let order = ToolbarMFU.orderedActions(
            counts: ["italic": 9999, "code": 9999]
        )
        XCTAssertEqual(order.last, ToolbarAction.pinnedLast)
    }

    func testMostUsedHoistsToPositionRightAfterPinnedFirst() {
        let order = ToolbarMFU.orderedActions(
            counts: ["code": 10, "italic": 1]
        )
        XCTAssertEqual(order[0], .newLine)
        XCTAssertEqual(order[1], .code)
    }

    func testStableTiebreakUsesDefaultOrderIndex() {
        // bold + italic both at 5; defaultOrder has bold ahead of
        // italic, so the tie has to resolve bold-first.
        let order = ToolbarMFU.orderedActions(
            counts: ["italic": 5, "bold": 5]
        )
        let boldIdx = order.firstIndex(of: .bold)!
        let italicIdx = order.firstIndex(of: .italic)!
        XCTAssertLessThan(boldIdx, italicIdx)
    }

    func testIgnoresCountsAgainstPinnedActions() {
        // Even if storage somehow contains counts for the pinned
        // slots, the slot position wins by virtue of the algorithm.
        let order = ToolbarMFU.orderedActions(counts: [
            "newLine": 9999,
            "done": 9999,
            "code": 1,
        ])
        XCTAssertEqual(order.first, .newLine)
        XCTAssertEqual(order.last, .done)
    }

    func testCardinalityPreserved() {
        let order = ToolbarMFU.orderedActions(counts: ["code": 10])
        XCTAssertEqual(order.count, ToolbarAction.defaultOrder.count)
        XCTAssertEqual(Set(order).count, ToolbarAction.defaultOrder.count)
    }

    func testEveryCaseAppearsInDefaultOrder() {
        // Catches the "added a new case to ToolbarAction but forgot
        // defaultOrder" footgun.
        let inDefault = Set(ToolbarAction.defaultOrder)
        for action in ToolbarAction.allCases {
            XCTAssertTrue(
                inDefault.contains(action),
                "ToolbarAction.defaultOrder is missing \(action.rawValue)"
            )
        }
    }

    // MARK: - Middle range (pinned-excluded)

    /// `orderedMiddleActions` is what the view layer feeds into the
    /// `UIScrollView`'s stack — `pinnedFirst` / `pinnedLast` are
    /// rendered as static buttons outside the scroll, so they must
    /// NOT appear here. Catches a regression where the middle stack
    /// would otherwise render the `+` (newLine) twice (once outside
    /// the scroll, once inside).
    func testOrderedMiddleActionsExcludesPinned() {
        let middle = ToolbarMFU.orderedMiddleActions(counts: [:])
        XCTAssertFalse(middle.contains(ToolbarAction.pinnedFirst))
        XCTAssertFalse(middle.contains(ToolbarAction.pinnedLast))
        // Cardinality: every non-pinned action makes the cut.
        let expectedCount = ToolbarAction.defaultOrder.count - 2
        XCTAssertEqual(middle.count, expectedCount)
    }

    func testOrderedMiddleActionsHonoursCounts() {
        // `code` is way more used than everything else; should land
        // first in the middle range (i.e. visually right after the
        // pinned `+`).
        let middle = ToolbarMFU.orderedMiddleActions(counts: ["code": 10])
        XCTAssertEqual(middle.first, .code)
    }

    /// Pinned + middle must reconstruct the full `orderedActions` list
    /// — keeps the two APIs from drifting out of sync.
    func testMiddlePlusPinnedEqualsOrderedActions() {
        let counts: [String: Int] = ["italic": 3, "bold": 2]
        let full = ToolbarMFU.orderedActions(counts: counts)
        let reconstructed =
            [ToolbarAction.pinnedFirst]
            + ToolbarMFU.orderedMiddleActions(counts: counts)
            + [ToolbarAction.pinnedLast]
        XCTAssertEqual(full, reconstructed)
    }
}
