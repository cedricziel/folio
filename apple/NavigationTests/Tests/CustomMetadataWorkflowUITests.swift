import CoreGraphics
import XCTest

final class CustomMetadataWorkflowUITests: XCTestCase {
    @MainActor
    func testChoosesMetadataTriggerAndExactValueCondition() {
        let app = openEditor(named: "Invoice amount")
        let trigger = app.buttons["metadata-workflow-trigger"]
        trigger.tap()
        app.buttons["When a custom field is created"].tap()
        XCTAssertFalse(app.switches["Match new value"].exists)
        trigger.tap()
        app.buttons["When custom field values change"].tap()
        app.buttons["metadata-workflow-field"].tap()
        app.buttons["Exact amount"].tap()
        enableNewValueComparison(in: app)
        let value = app.textFields["New value"]
        for _ in 0..<4 where !value.isHittable { app.swipeUp() }
        XCTAssertTrue(value.isHittable)
        value.tap()
        value.typeText("9007199254740993.01\n")
        saveWorkflow(named: "Invoice amount", in: app)
    }

    @MainActor
    func testAnyFieldTypeOmitsHiddenValueComparison() {
        let app = openEditor(named: "Any field change")
        app.buttons["metadata-workflow-trigger"].tap()
        app.buttons["When custom field values change"].tap()
        let kind = app.buttons["metadata-workflow-kind"]
        kind.tap()
        app.buttons["Decimal"].tap()
        enableNewValueComparison(in: app)
        kind.tap()
        app.buttons["Any type"].tap()
        XCTAssertFalse(app.switches["Match new value"].exists)
        saveWorkflow(named: "Any field change", in: app)
    }

    @MainActor
    private func openEditor(named name: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--custom-metadata-workflow"]
        app.launch()
        let create = app.buttons["New custom field workflow"]
        XCTAssertTrue(create.waitForExistence(timeout: 10))
        create.tap()
        XCTAssertTrue(app.buttons["metadata-workflow-trigger"].waitForExistence(timeout: 10))
        let nameField = app.textFields["Name"]
        nameField.tap()
        nameField.typeText(name + "\n")
        return app
    }

    @MainActor
    private func enableNewValueComparison(in app: XCUIApplication) {
        let compare = app.switches["Match new value"]
        for _ in 0..<4 where !compare.isHittable { app.swipeUp() }
        XCTAssertTrue(compare.isHittable)
        let trailingEdge = compare.coordinate(withNormalizedOffset: CGVector(dx: 1, dy: 0.5))
        let offThumb = trailingEdge.withOffset(CGVector(dx: -62, dy: 0))
        let onThumb = trailingEdge.withOffset(CGVector(dx: -32, dy: 0))
        offThumb.press(forDuration: 0.1, thenDragTo: onThumb)
        XCTAssertEqual(compare.value as? String, "1")
    }

    @MainActor
    private func saveWorkflow(named name: String, in app: XCUIApplication) {
        let url = app.textFields["Destination URL"]
        XCTAssertTrue(url.waitForExistence(timeout: 10))
        url.tap()
        url.typeText("http://127.0.0.1:9/unused\n")
        let save = app.buttons["metadata-workflow-save"]
        XCTAssertTrue(save.waitForExistence(timeout: 10))
        save.tap()
        XCTAssertTrue(app.staticTexts[name].waitForExistence(timeout: 10))
    }
}
