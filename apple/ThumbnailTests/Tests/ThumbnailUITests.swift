import XCTest

@MainActor final class ThumbnailUITests: XCTestCase {
    func testRowsWithPreviewsAndFallbacksRemainOpenable() {
        let app = XCUIApplication()
        app.launchArguments = ["--without-thumbnails"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Previews ready"].waitForExistence(timeout: 10), app.debugDescription)
        capture(app, name: "Placeholder baseline")
        app.terminate()
        app.launchArguments = []
        app.launch()
        let ready = app.staticTexts["Previews ready"]
        XCTAssertTrue(ready.waitForExistence(timeout: 10), app.debugDescription)
        capture(app, name: "Document previews and fallback icons")

        app.buttons["Not processed yet"].tap()
        XCTAssertTrue(app.staticTexts["Opened: Not processed yet"].waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["Authorization unavailable"].tap()
        XCTAssertTrue(
            app.staticTexts["Opened: Authorization unavailable"].waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["PDF preview"].tap()
        XCTAssertTrue(app.staticTexts["Opened: PDF preview"].waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["Image preview"].tap()
        XCTAssertTrue(app.staticTexts["Opened: Image preview"].waitForExistence(timeout: 5), app.debugDescription)
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = name
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
}
