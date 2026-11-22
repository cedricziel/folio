import XCTest

final class CustomMetadataWorkflowUITests: XCTestCase {
    @MainActor
    func testChoosesMetadataTriggerAndExactValueCondition() {
        let app = XCUIApplication()
        app.launchArguments = ["--custom-metadata-workflow"]
        app.launch()
        app.buttons["Open custom field workflow editor"].tap()
        let trigger = app.buttons["When"]
        XCTAssertTrue(trigger.waitForExistence(timeout: 10))
        trigger.tap()
        app.buttons["When a custom field is created"].tap()
        XCTAssertFalse(app.switches["Match new value"].exists)
        trigger.tap()
        app.buttons["When custom field values change"].tap()
        app.buttons["Custom field"].tap()
        app.buttons["Exact amount"].tap()
        let compare = app.switches["Match new value"]
        XCTAssertTrue(compare.waitForExistence(timeout: 10))
        compare.tap()
        let value = app.textFields["New value"]
        value.tap()
        value.typeText("9007199254740993.01")
        let name = app.textFields["Name"]
        name.tap()
        name.typeText("Invoice amount")
        let url = app.textFields["Destination URL"]
        url.tap()
        url.typeText("http://127.0.0.1:9/unused")
        app.buttons["Save workflow"].tap()
        XCTAssertTrue(app.staticTexts["Saved exact custom field workflow"].waitForExistence(timeout: 10))
    }
}
