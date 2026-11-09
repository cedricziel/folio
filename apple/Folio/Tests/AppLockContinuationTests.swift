import FolioKit
import Foundation
import XCTest

@MainActor
final class AppLockContinuationTests: XCTestCase {
    private var preferences: UserDefaults!
    private var suite: String!

    override func setUp() async throws {
        suite = "folio-lock-continuation-test-\(UUID().uuidString)"
        preferences = UserDefaults(suiteName: suite)!
    }

    override func tearDown() async throws {
        preferences.removePersistentDomain(forName: suite)
    }

    func testContinuationWaitsForUnlockAndReachesTheReceivingWindowOnce() async {
        var authenticates = true
        let lock = AppLock(defaults: preferences, authenticate: { _ in authenticates })
        await lock.setEnabled(true)
        lock.lockNow()
        let server = URL(string: "https://folio.example.com")!
        let session = SessionRestoration.signedIn(server)
        let route = HandoffRoute(vaultID: UUID(), target: .document(UUID()))
        let inbox = ContinuationInbox()
        let receivingWindow = UUID(), otherWindow = UUID()

        inbox.receive(
            .payload(HandoffPayload(server: server, vaultID: route.vaultID, target: route.target)),
            scene: receivingWindow)
        XCTAssertNil(inbox.take(scene: receivingWindow, isLocked: lock.isLocked, session: session))

        authenticates = false
        await lock.unlock()
        XCTAssertTrue(lock.isLocked)
        XCTAssertNil(inbox.take(scene: receivingWindow, isLocked: lock.isLocked, session: session))
        XCTAssertNotNil(inbox.pending)

        authenticates = true
        await lock.unlock()
        XCTAssertFalse(lock.isLocked)
        XCTAssertNil(inbox.take(scene: otherWindow, isLocked: lock.isLocked, session: session))
        XCTAssertEqual(inbox.take(scene: receivingWindow, isLocked: lock.isLocked, session: session), .open(route))
        XCTAssertNil(inbox.take(scene: receivingWindow, isLocked: lock.isLocked, session: session))
    }

    func testLaterContinuationReplacesTheHeldOne() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        await lock.setEnabled(true)
        lock.lockNow()
        let server = URL(string: "https://folio.example.com")!
        let window = UUID(), vault = UUID(), later = UUID()
        let inbox = ContinuationInbox()

        inbox.receive(.spotlight(DocumentLink(documentID: UUID(), vaultID: vault)), scene: window)
        inbox.receive(.spotlight(DocumentLink(documentID: later, vaultID: vault)), scene: window)
        await lock.unlock()

        XCTAssertEqual(
            inbox.take(scene: window, isLocked: lock.isLocked, session: .signedIn(server)),
            .open(HandoffRoute(vaultID: vault, target: .document(later))))
    }
}
