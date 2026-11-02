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
