import CoreGraphics
import XCTest

final class CustomMetadataWorkflowUITests: XCTestCase {
    @MainActor
    func testChoosesMetadataTriggerAndExactValueCondition() {
        let app = XCUIApplication()
        app.launchArguments = ["--custom-metadata-workflow"]
        app.launch()
        let create = app.buttons["New custom field workflow"]
        XCTAssertTrue(create.waitForExistence(timeout: 10))
        create.tap()
        let trigger = app.buttons["metadata-workflow-trigger"]
        XCTAssertTrue(trigger.waitForExistence(timeout: 10))
        app.textFields["Name"].tap()
        app.textFields["Name"].typeText("Invoice amount\n")
        trigger.tap()
        app.buttons["When a custom field is created"].tap()
        XCTAssertFalse(app.switches["Match new value"].exists)
        trigger.tap()
        app.buttons["When custom field values change"].tap()
        app.buttons["metadata-workflow-field"].tap()
        app.buttons["Exact amount"].tap()
        let compare = app.switches["Match new value"]
        for _ in 0..<4 where !compare.isHittable { app.swipeUp() }
        XCTAssertTrue(compare.isHittable)
        compare.coordinate(withNormalizedOffset: CGVector(dx: 0.92, dy: 0.5)).tap()
        XCTAssertEqual(compare.value as? String, "1")
        let value = app.textFields["New value"]
        for _ in 0..<4 where !value.isHittable { app.swipeUp() }
        XCTAssertTrue(value.isHittable)
        value.tap()
        value.typeText("9007199254740993.01")
        let url = app.textFields["Destination URL"]
        XCTAssertTrue(url.waitForExistence(timeout: 10))
        url.tap()
        url.typeText("http://127.0.0.1:9/unused")
        app.buttons["Save workflow"].tap()
        XCTAssertTrue(app.staticTexts["Invoice amount"].waitForExistence(timeout: 10))
    }
}
