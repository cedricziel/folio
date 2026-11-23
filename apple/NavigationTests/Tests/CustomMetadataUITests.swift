import CoreGraphics
import XCTest

final class CustomMetadataUITests: XCTestCase {
    @MainActor
    func testViewerCanFollowLogicalLinkWithoutEditControls() {
        let app = XCUIApplication()
        app.launchArguments = ["--custom-metadata", "--read-only"]
        app.launch()
        app.buttons["Open custom fields"].tap()
        let link = app.buttons["Linked receipt"]
        XCTAssertTrue(link.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["9007199254740993.01"].exists)
        XCTAssertFalse(app.buttons["Edit Exact amount"].exists)
        link.tap()
        XCTAssertTrue(app.staticTexts["Opened linked receipt"].waitForExistence(timeout: 10))
    }

    @MainActor
    func testEditorKeepsAnExactDecimalString() {
        let app = XCUIApplication()
        app.launchArguments = ["--custom-metadata"]
        app.launch()
        app.buttons["Open custom fields"].tap()
        let edit = app.buttons["Edit Exact amount"]
        XCTAssertTrue(edit.waitForExistence(timeout: 10))
        edit.tap()
        let value = app.textFields["Value"]
        XCTAssertTrue(value.waitForExistence(timeout: 10))
        value.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.5)).tap()
        value.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 19) + "9007199254740993.02")
        app.buttons["Save"].tap()
        XCTAssertTrue(app.staticTexts["9007199254740993.02"].waitForExistence(timeout: 10))
    }
}
