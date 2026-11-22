import XCTest

@MainActor final class WorkflowLifecycleUITests: XCTestCase {
    func testTrashAndRestoreRetainConfigurationAndFailedRunHistory() {
        let app = launch()
        openWorkflow(app)
        XCTAssertTrue(app.staticTexts["Created by, Taylor"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Updated by, Morgan"].exists, app.debugDescription)
        let failedRun = app.staticTexts["Receiver unavailable"]
        XCTAssertTrue(scroll(app, until: failedRun), app.debugDescription)
        capture(app, name: "Workflow configuration and failed run history")
        moveToTrash(app)
        XCTAssertTrue(app.staticTexts["No workflows"].waitForExistence(timeout: 5), app.debugDescription)
        showTrash(app)
        openWorkflow(app)
        XCTAssertTrue(app.staticTexts["State, In Trash"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(scroll(app, until: failedRun), app.debugDescription)
        capture(app, name: "Retained workflow history in Trash")
        let restore = app.buttons["Restore workflow"]
        XCTAssertTrue(scroll(app, until: restore), app.debugDescription)
        restore.tap()
        XCTAssertTrue(app.staticTexts["No workflows in Trash"].waitForExistence(timeout: 5), app.debugDescription)
        app.descendants(matching: .any).matching(identifier: "Active").firstMatch.tap()
        openWorkflow(app)
        XCTAssertTrue(scroll(app, until: failedRun), app.debugDescription)
        capture(app, name: "Restored workflow preserves failed run history")
    }

    func testListHistoryAndDeleteFailuresCanBeRetried() {
        let app = launch(arguments: ["--initial-load-error", "--workflow-history-error", "--workflow-delete-error"])
        let retry = app.buttons["Try again"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10), app.debugDescription)
        capture(app, name: "Workflow list load failure")
        retry.tap()
        openWorkflow(app)
        XCTAssertTrue(
            app.staticTexts["Couldn't load run history. Try again."].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(scroll(app, until: retry), app.debugDescription)
        retry.tap()
        XCTAssertTrue(app.staticTexts["Receiver unavailable"].waitForExistence(timeout: 5), app.debugDescription)
        moveToTrash(app)
        XCTAssertTrue(
            app.staticTexts["Couldn't move this workflow to Trash. Try again."].waitForExistence(timeout: 5),
            app.debugDescription)
        capture(app, name: "Failed workflow removal retains its detail")
        let deleteRetry = app.buttons["Try moving to Trash again"]
        XCTAssertTrue(scroll(app, until: deleteRetry), app.debugDescription)
        deleteRetry.tap()
        app.buttons["Move to Trash"].tap()
        XCTAssertTrue(app.staticTexts["No workflows"].waitForExistence(timeout: 5), app.debugDescription)
    }

    func testCatalogTriggerCanBeChangedAndReopened() {
        let app = launch()
        openWorkflow(app)
        let edit = app.buttons["Edit trigger"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5), app.debugDescription)
        edit.tap()
        let note = app.staticTexts["When a note is created"]
        XCTAssertTrue(note.waitForExistence(timeout: 5), app.debugDescription)
        note.tap()
        capture(app, name: "Workflow trigger selected from catalog")
        let save = app.buttons["Save"]
        XCTAssertTrue(save.isEnabled, app.debugDescription)
        save.tap()
        XCTAssertTrue(app.staticTexts["When a note is created"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(edit.waitForExistence(timeout: 5), app.debugDescription)
        edit.tap()
        let source = app.staticTexts["When a source becomes unavailable"]
        XCTAssertTrue(source.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(app.buttons["Save"].isEnabled, app.debugDescription)
        source.tap()
        app.buttons["Save"].tap()
        XCTAssertTrue(source.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(edit.waitForExistence(timeout: 5), app.debugDescription)
        capture(app, name: "Saved ingestion trigger on workflow")
    }

    func testNonOwnersCannotReadWorkflowConfigurationOrUseLifecycleActions() {
        let app = launch(arguments: ["--read-only"])
        XCTAssertTrue(app.staticTexts["Owner access required"].waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertFalse(app.staticTexts["Invoice notifications"].exists)
        XCTAssertFalse(app.buttons["Move workflow to Trash"].exists)
        XCTAssertFalse(app.buttons["Restore workflow"].exists)
        capture(app, name: "Workflow owner access restriction")
    }

    private func launch(arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--workflow-lifecycle"] + arguments
        app.launch()
        return app
    }

    private func openWorkflow(_ app: XCUIApplication) {
        let row = app.staticTexts["Invoice notifications"]
        XCTAssertTrue(row.waitForExistence(timeout: 10), app.debugDescription)
        row.tap()
    }

    private func showTrash(_ app: XCUIApplication) {
        let control = app.descendants(matching: .any).matching(identifier: "Trash").firstMatch
        XCTAssertTrue(control.waitForExistence(timeout: 5), app.debugDescription)
        control.tap()
    }

    private func moveToTrash(_ app: XCUIApplication) {
        let remove = app.buttons["Move workflow to Trash"]
        XCTAssertTrue(scroll(app, until: remove), app.debugDescription)
        remove.tap()
        let confirm = app.buttons["Move to Trash"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), app.debugDescription)
        confirm.tap()
    }

    private func scroll(_ app: XCUIApplication, until element: XCUIElement) -> Bool {
        for _ in 0..<8 {
            if element.exists && element.isHittable {
                return true
            }
            #if os(macOS)
                app.scrollViews.firstMatch.scroll(byDeltaX: 0, deltaY: -450)
            #else
                app.swipeUp()
            #endif
        }
        return element.exists && element.isHittable
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
