import XCTest

@MainActor final class NavigationTests: XCTestCase {
    func testInboxOpensAnotherDocumentAfterReturning() {
        let app = XCUIApplication()
        app.launch()
        let first = app.staticTexts["First fixture"]
        XCTAssertTrue(first.waitForExistence(timeout: 10), app.debugDescription)
        first.tap()
        XCTAssertTrue(app.navigationBars["First fixture"].waitForExistence(timeout: 5), app.debugDescription)
        app.navigationBars.buttons.element(boundBy: 0).tap()
        let second = app.staticTexts["Second fixture"]
        XCTAssertTrue(second.waitForExistence(timeout: 5), app.debugDescription)
        second.tap()
        XCTAssertTrue(app.navigationBars["Second fixture"].waitForExistence(timeout: 5), app.debugDescription)
    }
}
