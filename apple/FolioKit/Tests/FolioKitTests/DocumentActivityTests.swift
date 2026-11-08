import AppIntents
import Foundation
import Testing

@testable import FolioKit

private let serverA = URL(string: "https://a.folio.example")!
private let serverB = URL(string: "https://b.folio.example")!
private let vault = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!

private func summary(_ title: String, id: UUID = UUID()) -> DocumentSummary {
    DocumentSummary(id: id, title: title, date: Date(timeIntervalSince1970: 0), mimeType: "application/pdf")
}

@MainActor
private func signedInTracker(_ server: URL = serverA, handoff: Bool = false) -> DocumentActivityTracker {
    let tracker = DocumentActivityTracker(isEligibleForHandoff: handoff)
    tracker.sessionChanged(to: .signedIn(server))
    return tracker
}

@MainActor struct DocumentActivityTrackerTests {
    @Test func showingADocumentMakesItTheCurrentActivity() {
        let tracker = signedInTracker()
        let letter = summary("Household letter")

        tracker.show(letter, vaultID: vault)

        #expect(tracker.current == DocumentActivityPayload(server: serverA, vaultID: vault, documentID: letter.id))
        #expect(tracker.isCurrent(letter.id))
    }

    @Test func nothingIsCurrentWithoutASignedInSession() {
        let tracker = DocumentActivityTracker()
        tracker.show(summary("Before sign-in"), vaultID: vault)
        #expect(tracker.current == nil)

        tracker.sessionChanged(to: .interrupted(serverA))
        tracker.show(summary("Interrupted"), vaultID: vault)
        #expect(tracker.current == nil)
    }

    @Test func openingAnotherDocumentReplacesTheActivity() {
        let tracker = signedInTracker()
        let first = summary("First")
        let second = summary("Second")

        tracker.show(first, vaultID: vault)
        tracker.show(second, vaultID: vault)
        // The first view disappears after the second appeared, which must not clear the second.
        tracker.hide(documentID: first.id)

        #expect(tracker.current?.documentID == second.id)
        #expect(!tracker.isCurrent(first.id))
    }

    @Test func navigatingAwayClearsTheActivity() {
        let tracker = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.hide(documentID: letter.id)

        #expect(tracker.current == nil)
    }

    @Test func deletingTheDocumentClearsTheActivity() {
        let tracker = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.documentRemoved(letter.id)

        #expect(tracker.current == nil)
    }

    @Test(arguments: [SessionRestoration.signedOut, .expired(serverA), .signedIn(serverB)])
    func signOutExpiryOrAnotherServerForgetTheDocument(_ next: SessionRestoration) {
        let tracker = signedInTracker()
        tracker.show(summary("Letter"), vaultID: vault)

        tracker.sessionChanged(to: next)
        tracker.sessionChanged(to: .signedIn(serverA))

        #expect(tracker.current == nil)
    }

    @Test(arguments: [
        SessionRestoration.interrupted(serverA), .restoring, .unavailable(server: serverA, message: "keychain"),
    ])
    func anInterruptedSessionWithdrawsTheActivityAndRestoringItPublishesItAgain(_ pause: SessionRestoration) {
        let tracker = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.sessionChanged(to: pause)
        #expect(tracker.current == nil)

        tracker.sessionChanged(to: .signedIn(serverA))
        #expect(tracker.current == DocumentActivityPayload(server: serverA, vaultID: vault, documentID: letter.id))
    }

    @Test func signingInAgainOnTheSameServerForgetsThePreviousAccountsDocument() {
        let tracker = signedInTracker()
        tracker.show(summary("Account X"), vaultID: vault)
        tracker.sessionChanged(to: .interrupted(serverA))

        tracker.signedIn(to: serverA)
        tracker.sessionChanged(to: .signedIn(serverA))

        #expect(tracker.current == nil)
    }

    @Test func eachSceneTracksItsOwnDocument() {
        let left = signedInTracker()
        let right = signedInTracker()
        let first = summary("First")
        let second = summary("Second")

        left.show(first, vaultID: vault)
        right.show(second, vaultID: vault)

        #expect(left.isCurrent(first.id))
        #expect(right.isCurrent(second.id))
    }

    @Test func activityIdentifiesTheDocumentByIDsOnly() {
        let tracker = signedInTracker()
        let letter = summary("Household letter")
        tracker.show(letter, vaultID: vault)
        let activity = NSUserActivity(activityType: DocumentActivityPayload.activityType)

        tracker.configure(activity)

        #expect(activity.title == nil)
        #expect(DocumentActivityPayload(userInfo: activity.userInfo) == tracker.current)
        #expect(activity.requiredUserInfoKeys == DocumentActivityPayload.requiredKeys)
        #expect(!activity.isEligibleForPublicIndexing)
        #expect(!activity.isEligibleForSearch)
        #expect(!activity.isEligibleForHandoff)
        #expect(activity.targetContentIdentifier == letter.id.uuidString.lowercased())
        if #available(iOS 26, macOS 26, *) {
            #expect(
                activity.appEntityIdentifier
                    == EntityIdentifier(
                        for: DocumentEntity.self, identifier: DocumentEntityID(documentID: letter.id, vaultID: vault)))
        }
    }

    @Test func handoffIsOfferedOnlyWhenTheSceneAllowsItAndADocumentIsOpen() {
        let tracker = signedInTracker(handoff: true)
        let letter = summary("Lease")
        tracker.show(letter, vaultID: vault)
        let activity = NSUserActivity(activityType: DocumentActivityPayload.activityType)

        tracker.configure(activity)
        #expect(activity.isEligibleForHandoff)
        #expect(!activity.isEligibleForSearch)

        tracker.hide(documentID: letter.id)
        tracker.configure(activity)
        #expect(!activity.isEligibleForHandoff)
    }

    @Test func clearedActivityCarriesNoDocument() {
        let tracker = signedInTracker()
        tracker.show(summary("Household letter"), vaultID: vault)
        let activity = NSUserActivity(activityType: DocumentActivityPayload.activityType)
        tracker.configure(activity)

        tracker.sessionChanged(to: .signedOut)
        tracker.configure(activity)

        #expect(activity.userInfo?.isEmpty ?? true)
        #expect(activity.targetContentIdentifier == nil)
        if #available(iOS 26, macOS 26, *) {
            #expect(activity.appEntityIdentifier == nil)
        }
    }

    @Test func deletingThroughTheListClearsTheActivityIncludingARedo() async {
        let document = Document(title: "Letter", content: "Read", ocrStatus: .done, embedStatus: .done)
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let tracker = signedInTracker()
        list.onRemove = { tracker.documentRemoved($0) }
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        let shown = DocumentSummary(document, correspondent: nil, documentType: nil)

        tracker.show(shown, vaultID: vault)
        #expect(await history.moveToTrash(document, undoManager: manager))
        #expect(tracker.current == nil)

        manager.undo()
        await history.waitForPendingOperations()
        tracker.show(shown, vaultID: vault)
        #expect(tracker.isCurrent(document.id))

        manager.redo()
        await history.waitForPendingOperations()
        #expect(tracker.current == nil)
    }
}

struct DocumentActivityPayloadTests {
    @Test func payloadRoundTripsThroughUserInfo() {
        let payload = DocumentActivityPayload(server: serverA, vaultID: vault, documentID: UUID())
        #expect(DocumentActivityPayload(userInfo: payload.userInfo) == payload)
    }

    @Test func payloadCarriesOnlyIdentifiers() {
        let payload = DocumentActivityPayload(server: serverA, vaultID: vault, documentID: UUID())
        #expect(Set(payload.userInfo.keys.compactMap { $0 as? String }) == ["version", "server", "vault", "document"])
    }

    @Test func malformedPayloadsAreRejected() {
        let valid = DocumentActivityPayload(server: serverA, vaultID: vault, documentID: UUID()).userInfo
        #expect(DocumentActivityPayload(userInfo: nil) == nil)
        for key in ["version", "server", "vault", "document"] {
            var broken = valid
            broken[key] = "nonsense"
            #expect(DocumentActivityPayload(userInfo: broken) == nil, "corrupt \(key)")
            broken.removeValue(forKey: key)
            #expect(DocumentActivityPayload(userInfo: broken) == nil, "missing \(key)")
        }
        var future = valid
        future["version"] = DocumentActivityPayload.version + 1
        #expect(DocumentActivityPayload(userInfo: future) == nil)
    }
}
