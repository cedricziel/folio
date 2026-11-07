import XCTest

@MainActor final class ClassificationUITests: XCTestCase {
    func testConflictKeepsOriginalThenRestoresAfterReplacementIsRemoved() {
        let app = launch()
        XCTAssertTrue(app.buttons["Move Insurance to Trash"].waitForExistence(timeout: 10), app.debugDescription)
        capture(app, name: "Active and trashed classifications")
        let history = app.descendants(matching: .any).matching(identifier: "History").firstMatch
        XCTAssertTrue(history.waitForExistence(timeout: 5), app.debugDescription)
        history.tap()
        XCTAssertTrue(app.staticTexts["Created by Morgan"].waitForExistence(timeout: 5), app.debugDescription)
        capture(app, name: "Active classification audit metadata")
        history.tap()

        let restore = app.buttons["Restore"].firstMatch
        XCTAssertTrue(restore.waitForExistence(timeout: 5), app.debugDescription)
        capture(app, name: "Retained classification in Trash")
        restore.tap()
        let conflict = app.staticTexts["Restore failed. An active entry already uses this name."]
        XCTAssertTrue(conflict.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Move Insurance to Trash"].exists)
        XCTAssertTrue(app.buttons["Restore"].firstMatch.exists)
        capture(app, name: "Restore conflict preserves both classifications")

        app.buttons["Move Insurance to Trash"].tap()
        let confirmation = app.buttons["Move to Trash"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5), app.debugDescription)
        confirmation.tap()
        XCTAssertTrue(
            app.staticTexts["Replacement Insurance moved to Trash"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(
            app.buttons.matching(identifier: "Restore").element(boundBy: 1).waitForExistence(timeout: 5),
            app.debugDescription)
        app.buttons["Restore"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Original Insurance restored"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Move Insurance to Trash"].waitForExistence(timeout: 5), app.debugDescription)
        let restoredHistory = app.descendants(matching: .any).matching(identifier: "History").firstMatch
        restoredHistory.tap()
        XCTAssertTrue(app.staticTexts["Created by Taylor"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(conflict.exists)
        capture(app, name: "Original identity and creation attribution restored")
    }

    func testInitialLoadFailureCanBeRetried() {
        let app = launch(arguments: ["--initial-load-error"])
        XCTAssertTrue(app.buttons["Retry"].waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Couldn't reach the server."].exists)
        capture(app, name: "Classification load error and Retry")
        app.buttons["Retry"].tap()
        XCTAssertTrue(app.buttons["Move Insurance to Trash"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Restore"].firstMatch.exists)
        XCTAssertFalse(app.buttons["Retry"].exists)
        capture(app, name: "Classifications after successful Retry")
    }

    func testReadOnlyModeShowsHistoryAndTrashWithoutMutationControls() {
        let app = launch(arguments: ["--read-only"])
        XCTAssertTrue(app.staticTexts["Insurance"].firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Deleted"].firstMatch.exists)
        XCTAssertFalse(app.buttons["Add tag"].exists)
        XCTAssertFalse(app.buttons["Rename Insurance"].exists)
        XCTAssertFalse(app.buttons["Move Insurance to Trash"].exists)
        XCTAssertFalse(app.buttons["Restore"].firstMatch.exists)
        capture(app, name: "Read-only active classifications and Trash")
    }

    func testLargeTextKeepsClassificationActionsReachable() {
        let app = launch(arguments: ["--large-text"])
        let name = "Insurance and household correspondence"
        let remove = app.buttons["Move \(name) to Trash"]
        XCTAssertTrue(remove.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(remove.isHittable, app.debugDescription)
        capture(app, name: "Classification layout with accessibility text")
        #if os(macOS)
            app.scrollViews.firstMatch.scroll(byDeltaX: 0, deltaY: -500)
        #else
            app.swipeUp()
        #endif
        let restore = app.buttons["Restore"].firstMatch
        XCTAssertTrue(restore.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(restore.isHittable, app.debugDescription)
        capture(app, name: "Trash actions with accessibility text")
    }

    private func launch(arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = arguments
        app.launch()
        return app
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = name
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
}
