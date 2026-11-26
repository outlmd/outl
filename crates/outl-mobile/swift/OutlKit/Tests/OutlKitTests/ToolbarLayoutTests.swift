import XCTest
@testable import OutlKit

final class ToolbarLayoutTests: XCTestCase {

    func testStackIsTheCapsuleAndItsMargins() {
        XCTAssertEqual(ToolbarLayout.stackHeight, 56)
        XCTAssertEqual(
            ToolbarLayout.stackHeight,
            ToolbarLayout.capsuleHeight + 2 * ToolbarLayout.verticalMargin
        )
    }

    func testClearanceIsPublishedAtTheStackHeight() {
        let js = ToolbarLayout.showClearanceScript
        XCTAssertTrue(js.contains("'--outl-toolbar-clearance','56px'"))
    }

    func testClearancePropertyMatchesTheStylesheet() {
        XCTAssertEqual(ToolbarLayout.clearanceProperty, "--outl-toolbar-clearance")
    }

    func testShowRevealsOnlyATextareaAndOnlyIfStillFocused() {
        let js = ToolbarLayout.showClearanceScript
        XCTAssertTrue(js.contains("a.tagName!=='TEXTAREA'"))
        XCTAssertTrue(js.contains("a===document.activeElement"))
        XCTAssertTrue(js.contains("block:'nearest'"))
    }

    func testHideRemovesTheProperty() {
        XCTAssertEqual(
            ToolbarLayout.hideClearanceScript,
            "document.documentElement.style.removeProperty('--outl-toolbar-clearance')"
        )
    }

    func testEditorProbeExcludesDialogs() {
        XCTAssertTrue(ToolbarLayout.editorFocusedScript.contains("[role=\"dialog\"]"))
    }

    func testEditorAnswerFailsOpen() {
        XCTAssertTrue(ToolbarLayout.isEditorFocused(true))
        XCTAssertFalse(ToolbarLayout.isEditorFocused(false))
        XCTAssertTrue(ToolbarLayout.isEditorFocused(nil))
        XCTAssertTrue(ToolbarLayout.isEditorFocused("unexpected"))
    }
}
