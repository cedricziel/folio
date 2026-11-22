import XCTest

@MainActor final class VaultLifecycleUITests: XCTestCase {
    func testOwnerSeesAttributionMembersAndHistory() {
        let app = launch()
        let vault = app.buttons.containing(.staticText, identifier: "Household").firstMatch
        XCTAssertTrue(vault.waitForExistence(timeout: 10), app.debugDescription)
        vault.tap()
        XCTAssertTrue(app.staticTexts["Created by, Morgan"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Save name"].exists, app.debugDescription)
        scrollTo(app.buttons["Grant access"], in: app)
        XCTAssertTrue(app.buttons["Change role"].exists, app.debugDescription)
        capture(app, name: "Vault owner metadata and member grants")
        scrollTo(app.staticTexts["Created Household"], in: app)
        XCTAssertTrue(app.staticTexts["Created Household"].exists, app.debugDescription)
        capture(app, name: "Vault owner history")
    }

    func testViewerSeesMetadataWithoutOwnerActions() {
        let app = launch(["--read-only"])
        let vault = app.buttons.containing(.staticText, identifier: "Household").firstMatch
        XCTAssertTrue(vault.waitForExistence(timeout: 10), app.debugDescription)
        vault.tap()
        XCTAssertTrue(app.staticTexts["Updated by, Taylor"].waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(app.buttons["Save name"].exists)
        XCTAssertFalse(app.buttons["Grant access"].exists)
        XCTAssertFalse(app.buttons["Delete vault"].exists)
        XCTAssertFalse(app.staticTexts["Created Household"].exists)
        capture(app, name: "Vault viewer lifecycle metadata")
    }

    func testDeletedVaultRestoresFromEmptyCatalog() {
        let app = launch(["--vault-empty"])
        XCTAssertTrue(
            app.staticTexts["No active vaults. Create a vault or restore one below to open your library."]
                .waitForExistence(timeout: 10), app.debugDescription)
        let vault = app.buttons.containing(.staticText, identifier: "Household").firstMatch
        XCTAssertTrue(vault.waitForExistence(timeout: 5), app.debugDescription)
        capture(app, name: "Empty library with vault recovery")
        vault.tap()
        let restore = app.buttons["Restore vault"].firstMatch
        XCTAssertTrue(restore.waitForExistence(timeout: 5), app.debugDescription)
        restore.tap()
        let confirmation = app.buttons["vault-restore-confirmation"].firstMatch
        guard confirmation.waitForExistence(timeout: 5) else {
            XCTFail("The restore confirmation action is missing.\n\(app.debugDescription)")
            return
        }
        capture(app, name: "Vault restore confirmation")
        confirmation.tap()
        XCTAssertTrue(app.buttons["Save name"].waitForExistence(timeout: 5), app.debugDescription)
        scrollTo(app.staticTexts["Restored Household"], in: app)
        XCTAssertTrue(app.staticTexts["Restored Household"].exists, app.debugDescription)
        capture(app, name: "Restored vault retained history")
    }

    private func launch(_ arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--vault-lifecycle"] + arguments
        app.launch()
        return app
    }

    private func scrollTo(_ element: XCUIElement, in app: XCUIApplication) {
        for _ in 0..<6 {
            if element.exists && element.isHittable { return }
            #if os(macOS)
                app.scrollViews.firstMatch.scroll(byDeltaX: 0, deltaY: -500)
            #else
                app.swipeUp()
            #endif
        }
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
