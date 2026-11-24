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
        let note = app.buttons["When a note is created"]
        XCTAssertTrue(note.waitForExistence(timeout: 5), app.debugDescription)
        note.tap()
        capture(app, name: "Workflow trigger selected from catalog")
        let save = app.buttons["Save"]
        XCTAssertTrue(save.isEnabled, app.debugDescription)
        save.tap()
        XCTAssertTrue(save.waitForNonExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["When a note is created"].waitForExistence(timeout: 5), app.debugDescription)
        edit.tap()
        let comment = app.buttons["When a comment is created"]
        XCTAssertTrue(comment.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(note.isSelected, app.debugDescription)
        XCTAssertFalse(save.isEnabled, app.debugDescription)
        comment.tap()
        XCTAssertTrue(save.isEnabled, app.debugDescription)
        save.tap()
        XCTAssertTrue(save.waitForNonExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["When a comment is created"].waitForExistence(timeout: 5), app.debugDescription)

        edit.tap()
        let source = app.buttons["When a source becomes unavailable"]
        XCTAssertTrue(source.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(comment.isSelected, app.debugDescription)
        XCTAssertFalse(save.isEnabled, app.debugDescription)
        source.tap()
        save.tap()
        XCTAssertTrue(save.waitForNonExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(
            app.staticTexts["When a source becomes unavailable"].waitForExistence(timeout: 5), app.debugDescription)
        capture(app, name: "Saved ingestion trigger on workflow")
    }

    func testTriggerConflictCanReloadAndSaveWithoutReenablingTheWorkflow() {
        let app = launch(arguments: ["--concurrent-workflow-edit"])
        openWorkflow(app)
        let edit = app.buttons["Edit trigger"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5), app.debugDescription)
        edit.tap()
        let note = app.buttons["When a note is created"]
        XCTAssertTrue(note.waitForExistence(timeout: 5), app.debugDescription)
        note.tap()
        let save = app.buttons["Save"]
        save.tap()
        let reload = app.buttons["Reload workflow"]
        XCTAssertTrue(reload.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(save.isEnabled, app.debugDescription)
        reload.tap()
        XCTAssertTrue(reload.waitForNonExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(save.isEnabled, app.debugDescription)
        note.tap()
        save.tap()
        XCTAssertTrue(save.waitForNonExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["State, Off"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["When a note is created"].exists, app.debugDescription)
    }

    func testOrderedWebhookActionsAreSavedAndReopened() {
        let app = launch(arguments: ["--workflow-action-order"])
        openWorkflow(app)
        let edit = app.buttons["Edit actions"]
        guard edit.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        edit.tap()
        let first = app.buttons["workflow-action-1"]
        guard first.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        XCTAssertTrue(first.label.contains("https://first.test/hook"), app.debugDescription)
        let moveUp = app.buttons["Move action 2 up"]
        guard scroll(app, until: moveUp) else { return XCTFail(app.debugDescription) }
        moveUp.tap()
        XCTAssertTrue(first.label.contains("https://second.test/hook"), app.debugDescription)
        capture(app, name: "Reordered workflow actions")

        let save = app.navigationBars["Edit actions"].buttons["Save"]
        save.tap()
        guard save.waitForNonExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        edit.tap()
        guard first.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        XCTAssertTrue(first.label.contains("https://second.test/hook"), app.debugDescription)
    }

    func testWorkflowCanBeCreatedAndEnabled() {
        verifyWorkflowCreation(withAdditionalTrigger: false)
    }

    func testOrderedTriggersSurviveCreationAndEnabling() {
        verifyWorkflowCreation(withAdditionalTrigger: true)
    }

    private func verifyWorkflowCreation(withAdditionalTrigger: Bool) {
        let argument = withAdditionalTrigger ? "--workflow-create-multiple" : "--workflow-create"
        let app = launch(arguments: [argument])
        let newWorkflow = app.buttons["New workflow"]
        guard newWorkflow.waitForExistence(timeout: 10),
            app.staticTexts["No workflows"].waitForExistence(timeout: 5)
        else { return XCTFail(app.debugDescription) }
        newWorkflow.tap()
        let create = app.navigationBars["New workflow"].buttons["Create"]
        XCTAssertFalse(create.isEnabled, app.debugDescription)
        let name = app.textFields["Name"]
        name.tap()
        name.typeText("Notify intake")
        let trigger = app.buttons["workflow-new-trigger"]
        guard scroll(app, until: trigger) else { return XCTFail(app.debugDescription) }
        trigger.tap()
        let uploaded = app.buttons["When a document is added"]
        guard uploaded.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        uploaded.tap()
        if withAdditionalTrigger {
            let addTrigger = app.buttons["Add trigger"]
            guard scroll(app, until: addTrigger) else { return XCTFail(app.debugDescription) }
            addTrigger.tap()
            let additional = app.buttons["workflow-additional-trigger-1"]
            guard scroll(app, until: additional) else { return XCTFail(app.debugDescription) }
            additional.tap()
            let note = app.buttons["When a note is created"]
            guard note.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
            note.tap()
            let up = app.buttons["Move up, additional trigger 1"]
            guard scroll(app, until: up) else { return XCTFail(app.debugDescription) }
            up.tap()
        }
        let add = app.buttons["Add action"]
        guard scroll(app, until: add) else { return XCTFail(app.debugDescription) }
        add.tap()
        let webhook = app.buttons["Webhook"]
        guard webhook.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        webhook.tap()
        let action = app.buttons["workflow-action-1"]
        guard scroll(app, until: action) else { return XCTFail(app.debugDescription) }
        action.tap()
        let url = app.textFields["Webhook URL"]
        guard url.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        url.tap()
        url.typeText("https://new.test/hook")
        app.navigationBars["Webhook"].buttons.firstMatch.tap()
        guard create.waitForExistence(timeout: 5), create.isEnabled else { return XCTFail(app.debugDescription) }
        capture(app, name: "Ready to create a workflow")
        create.tap()

        let row = app.staticTexts["Notify intake"]
        guard row.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        row.tap()
        XCTAssertTrue(app.staticTexts["State, Off"].waitForExistence(timeout: 5), app.debugDescription)
        let settings = app.buttons["Edit settings"]
        guard settings.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        settings.tap()
        let enabled = app.switches["Enabled"]
        guard enabled.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        let toggle = enabled.switches.firstMatch
        guard toggle.exists else { return XCTFail(app.debugDescription) }
        toggle.tap()
        XCTAssertEqual(enabled.value as? String, "1", app.debugDescription)
        let save = app.navigationBars["Edit workflow settings"].buttons["Save"]
        guard save.isEnabled else { return XCTFail(app.debugDescription) }
        save.tap()
        guard save.waitForNonExistence(timeout: 5),
            app.staticTexts["State, On"].waitForExistence(timeout: 5)
        else { return XCTFail(app.debugDescription) }
        capture(app, name: "Created and enabled workflow")
    }

    func testRecurringScheduleCanBeChangedToOnceAndReopened() {
        let app = launch(arguments: ["--workflow-schedule"])
        openWorkflow(app)
        let edit = app.buttons["Edit trigger"]
        guard edit.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        edit.tap()

        let start = app.textFields["Start in UTC"]
        let interval = app.textFields["Repeat every (seconds)"]
        guard scroll(app, until: interval) else { return XCTFail(app.debugDescription) }
        XCTAssertEqual(start.value as? String, "2032-02-29T09:30:00.123456789Z", app.debugDescription)
        XCTAssertEqual(interval.value as? String, "3600", app.debugDescription)
        let once = app.buttons["Once"]
        guard scroll(app, until: once) else { return XCTFail(app.debugDescription) }
        once.tap()
        XCTAssertTrue(interval.waitForNonExistence(timeout: 5), app.debugDescription)

        let save = app.navigationBars["Edit trigger"].buttons["Save"]
        guard save.isEnabled else { return XCTFail(app.debugDescription) }
        save.tap()
        guard save.waitForNonExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        edit.tap()
        guard scroll(app, until: start) else { return XCTFail(app.debugDescription) }
        XCTAssertEqual(start.value as? String, "2032-02-29T09:30:00.123456789Z", app.debugDescription)
        XCTAssertFalse(interval.exists, app.debugDescription)
        capture(app, name: "Saved one-time UTC schedule")
    }

    func testDocumentDateActionCanBeAddedReorderedAndReopened() {
        let app = launch(arguments: ["--workflow-document-actions"])
        openWorkflow(app)
        let edit = app.buttons["Edit actions"]
        guard edit.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        edit.tap()

        let add = app.buttons["Add action"]
        guard scroll(app, until: add) else { return XCTFail(app.debugDescription) }
        add.tap()
        let chooseDate = app.buttons["Set document date"]
        guard scroll(app, until: chooseDate) else { return XCTFail(app.debugDescription) }
        chooseDate.tap()
        let second = app.buttons["workflow-action-2"]
        guard scroll(app, until: second) else { return XCTFail(app.debugDescription) }
        second.tap()

        let date = app.textFields["workflow-document-date"]
        guard date.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        XCTAssertEqual(date.label, "Document date")
        date.tap()
        date.typeText("2028-02-29")
        let back = app.navigationBars["Set document date"].buttons.firstMatch
        guard back.waitForExistence(timeout: 5), back.isHittable else { return XCTFail(app.debugDescription) }
        back.tap()

        let moveUp = app.buttons["Move action 2 up"]
        guard scroll(app, until: moveUp) else { return XCTFail(app.debugDescription) }
        moveUp.tap()
        let first = app.buttons["workflow-action-1"]
        XCTAssertTrue(first.label.contains("Set document date"), app.debugDescription)

        let save = app.navigationBars["Edit actions"].buttons["Save"]
        save.tap()
        guard save.waitForNonExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        edit.tap()
        guard first.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        first.tap()
        guard date.waitForExistence(timeout: 5) else { return XCTFail(app.debugDescription) }
        XCTAssertEqual(date.value as? String, "2028-02-29")
        capture(app, name: "Saved workflow date action")
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
