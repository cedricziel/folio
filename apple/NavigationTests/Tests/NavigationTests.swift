import XCTest

@MainActor final class NavigationTests: XCTestCase {
    func testInboxOpensAnotherDocumentAfterReturning() {
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launch()
        let first = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "First fixture")).firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 10), app.debugDescription)
        first.tap()
        XCTAssertTrue(app.navigationBars["First fixture"].waitForExistence(timeout: 5), app.debugDescription)
        app.navigationBars.buttons.element(boundBy: 0).tap()
        let second = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Second fixture")).firstMatch
        XCTAssertTrue(second.waitForExistence(timeout: 5), app.debugDescription)
        second.tap()
        XCTAssertTrue(app.navigationBars["Second fixture"].waitForExistence(timeout: 5), app.debugDescription)
    }

    func testWideWindowKeepsDocumentListBesidePreview() throws {
        XCUIDevice.shared.orientation = .landscapeLeft
        let app = XCUIApplication()
        app.launch()
        try XCTSkipIf(app.windows.firstMatch.frame.width < 1000, "Requires a full-width iPad window.")
        let first = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "First fixture")).firstMatch
        let second = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Second fixture")).firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 10), app.debugDescription)
        first.tap()
        XCTAssertTrue(app.staticTexts["first.pdf"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(second.isHittable, "The document list must remain available alongside the preview.")
        second.tap()
        XCTAssertTrue(app.staticTexts["second.pdf"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(first.isHittable)
    }

    func testPendingScanSurvivesSwipeDismissal() {
        let app = openScanReview()
        let title = app.navigationBars["Review a scan"]
        title.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.95)))
        XCTAssertTrue(title.exists, "An unfiled scan must remain available after a dismissal gesture.")
    }

    func testKeepingScanAfterDiscardRetainsReview() {
        let app = openScanReview()
        app.buttons["Discard"].tap()
        XCTAssertTrue(app.buttons["Discard Scan"].waitForExistence(timeout: 3))
        app.buttons["Keep Scan"].tap()
        XCTAssertTrue(app.navigationBars["Review a scan"].exists)
        XCTAssertTrue(app.buttons["File it"].exists)
    }

    func testConfirmingDiscardClosesScanReview() {
        let app = openScanReview()
        app.buttons["Discard"].tap()
        let discard = app.buttons["Discard Scan"]
        XCTAssertTrue(discard.waitForExistence(timeout: 3))
        discard.tap()
        XCTAssertTrue(app.buttons["Review fixture scan"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.navigationBars["Review a scan"].exists)
    }

    func testDocumentNotesSupportAddEditDeleteAndReturnToPreview() {
        let app = openDiscussion()
        app.buttons["Add note"].tap()
        let editor = app.textViews["Note text"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5), app.debugDescription)
        editor.tap()
        editor.typeText("Fixture note")
        XCTAssertTrue(app.buttons["Save"].isEnabled)
        app.buttons["Save"].tap()
        XCTAssertTrue(app.staticTexts["Fixture note"].waitForExistence(timeout: 5), app.debugDescription)

        app.buttons["document-entry-edit"].tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        editor.tap()
        editor.typeText(" revised")
        app.buttons["Save"].tap()
        let revised = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "revised")).firstMatch
        XCTAssertTrue(revised.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Updated"].exists)

        app.buttons["document-entry-delete"].tap()
        let confirm = app.buttons["confirm-entry-delete"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), app.debugDescription)
        confirm.tap()
        XCTAssertTrue(revised.waitForNonExistence(timeout: 5), app.debugDescription)
        app.buttons["Done"].tap()
        XCTAssertTrue(app.navigationBars["First fixture"].waitForExistence(timeout: 5), app.debugDescription)
    }

    func testLostCommentResponseRetainsDraftAndRetryCreatesOneEntry() {
        let app = openDiscussion(arguments: ["--discussion-save-failure"])
        app.segmentedControls.buttons["Comments"].tap()
        app.buttons["Add comment"].tap()
        let editor = app.textViews["Comment text"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5), app.debugDescription)
        editor.tap()
        editor.typeText("Keep this comment")
        app.buttons["Save"].tap()
        let retry = app.buttons["Retry save"]
        XCTAssertTrue(retry.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Couldn't reach the server."].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(retry.isEnabled)
        XCTAssertEqual(editor.value as? String, "Keep this comment")
        XCTAssertFalse(editor.isEnabled)
        XCTAssertTrue(app.buttons["Cancel"].isEnabled)
        XCTAssertFalse(app.buttons["Done"].isEnabled)
        retry.tap()
        XCTAssertTrue(app.staticTexts["Keep this comment"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertEqual(app.staticTexts.matching(identifier: "Keep this comment").count, 1)
    }

    private func openDiscussion(arguments: [String] = []) -> XCUIApplication {
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = arguments
        app.launch()
        let first = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "First fixture")).firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 10), app.debugDescription)
        first.tap()
        let discussion = app.buttons["Notes and comments"]
        XCTAssertTrue(discussion.waitForExistence(timeout: 5), app.debugDescription)
        discussion.tap()
        XCTAssertTrue(app.navigationBars["Notes and comments"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Add note"].waitForExistence(timeout: 5), app.debugDescription)
        return app
    }

    private func openScanReview() -> XCUIApplication {
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = ["--scan-review"]
        app.launch()
        app.buttons["Review fixture scan"].tap()
        XCTAssertTrue(app.navigationBars["Review a scan"].waitForExistence(timeout: 5))
        return app
    }
}
