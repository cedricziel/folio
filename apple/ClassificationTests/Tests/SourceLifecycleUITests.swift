import XCTest

@MainActor final class SourceLifecycleUITests: XCTestCase {
    func testDisableTrashAndRestoreRetainActivityAndAttribution() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Mailbox"].firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        app.staticTexts["Mailbox"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["By Taylor"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["By Morgan"].exists)
        capture(app, name: "Source lifecycle and import activity")
        app.buttons["Disable source"].tap()
        XCTAssertTrue(app.buttons["Enable source"].waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["Move source to Trash"].tap()
        let confirmation = app.buttons["Move to Trash"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5), app.debugDescription)
        confirmation.tap()
        showTrash(app)
        XCTAssertTrue(app.staticTexts["Mailbox"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        app.staticTexts["Mailbox"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Restore source"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["The document was saved."].exists)
        capture(app, name: "Trashed source retains import activity")
        app.buttons["Restore source"].tap()
        showActive(app)
        XCTAssertTrue(app.staticTexts["Mailbox"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        app.staticTexts["Mailbox"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Enable source"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts["By Taylor"].exists)
        capture(app, name: "Restored disabled source keeps creation actor")
    }

    func testRestoreConflictCanBeResolvedAndRetried() {
        let app = launch(arguments: ["--source-conflict"])
        showTrash(app)
        XCTAssertTrue(app.staticTexts["Archived mailbox"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        app.staticTexts["Archived mailbox"].firstMatch.tap()
        XCTAssertTrue(
            app.staticTexts["By System or unknown"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["Restore source"].tap()
        let error = app.staticTexts[
            "Another active source uses this name or occupies the Paperless slot. Resolve the conflict in web settings, then try again."
        ]
        XCTAssertTrue(error.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Restore source"].exists)
        capture(app, name: "Source restore conflict keeps retained source")
        app.buttons["Resolve fixture source conflict"].tap()
        app.buttons["Restore source"].tap()
        XCTAssertTrue(app.staticTexts["No sources in Trash"].waitForExistence(timeout: 5), app.debugDescription)
        showActive(app)
        XCTAssertTrue(app.staticTexts["Archived mailbox"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        app.staticTexts["Archived mailbox"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Enable source"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(error.exists)
        capture(app, name: "Source restore succeeds after conflict resolution")
    }

    func testConfiguredSourceHasNoMutationControlsEvenForAnOwner() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Server folder"].firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        app.staticTexts["Server folder"].firstMatch.tap()
        XCTAssertTrue(
            app.staticTexts["Configured on the server. Change this source in the server configuration."]
                .waitForExistence(timeout: 5),
            app.debugDescription)
        XCTAssertFalse(app.buttons["Disable source"].exists)
        XCTAssertFalse(app.buttons["Move source to Trash"].exists)
        capture(app, name: "Configured source is read only")
    }

    func testReadOnlyManagedSourceShowsAttributionWithoutMutationControls() {
        let app = launch(arguments: ["--read-only"])
        XCTAssertTrue(app.staticTexts["Mailbox"].firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        app.staticTexts["Mailbox"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["By Taylor"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(app.buttons["Disable source"].exists)
        XCTAssertFalse(app.buttons["Move source to Trash"].exists)
        capture(app, name: "Read-only managed source retains attribution and activity")
    }

    func testLoadFailureCanBeRetried() {
        let app = launch(arguments: ["--initial-load-error"])
        let retry = app.buttons["Try again"]
        XCTAssertTrue(retry.waitForExistence(timeout: 5), app.debugDescription)
        capture(app, name: "Source list retry")
        retry.tap()
        XCTAssertTrue(app.staticTexts["Mailbox"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
    }

    private func showActive(_ app: XCUIApplication) {
        let control = app.descendants(matching: .any).matching(identifier: "Active").firstMatch
        XCTAssertTrue(control.waitForExistence(timeout: 5), app.debugDescription)
        control.tap()
    }

    private func showTrash(_ app: XCUIApplication) {
        let control = app.descendants(matching: .any).matching(identifier: "Trash").firstMatch
        XCTAssertTrue(control.waitForExistence(timeout: 5), app.debugDescription)
        control.tap()
    }

    private func launch(arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--source-lifecycle"] + arguments
        app.launch()
        #if os(iOS)
            XCTAssertTrue(app.navigationBars["Ingestion activity"].waitForExistence(timeout: 10), app.debugDescription)
        #endif
        return app
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
