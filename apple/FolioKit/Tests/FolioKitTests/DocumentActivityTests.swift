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
private func signedInTracker(_ server: URL = serverA) -> (DocumentActivityTracker, DocumentLocator) {
    let locator = DocumentLocator()
    let tracker = DocumentActivityTracker(locator: locator)
    tracker.sessionChanged(to: .signedIn(server))
    return (tracker, locator)
}

@MainActor struct DocumentActivityTrackerTests {
    @Test func showingADocumentMakesItTheCurrentActivity() {
        let (tracker, locator) = signedInTracker()
        let letter = summary("Household letter")

        tracker.show(letter, vaultID: vault)

        #expect(tracker.current == DocumentActivityPayload(server: serverA, vaultID: vault, documentID: letter.id))
        #expect(tracker.isCurrent(letter.id))
        #expect(locator.vault(for: letter.id) == vault)
    }

    @Test func nothingIsCurrentWithoutASignedInSession() {
        let tracker = DocumentActivityTracker(locator: DocumentLocator())
        tracker.show(summary("Before sign-in"), vaultID: vault)
        #expect(tracker.current == nil)

        tracker.sessionChanged(to: .interrupted(serverA))
        tracker.show(summary("Interrupted"), vaultID: vault)
        #expect(tracker.current == nil)
    }

    @Test func openingAnotherDocumentReplacesTheActivity() {
        let (tracker, _) = signedInTracker()
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
        let (tracker, _) = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.hide(documentID: letter.id)

        #expect(tracker.current == nil)
        #expect(!tracker.isCurrent(letter.id))
    }

    @Test func deletingTheDocumentClearsTheActivityAndItsVault() {
        let (tracker, locator) = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.documentRemoved(letter.id)

        #expect(tracker.current == nil)
        #expect(locator.vault(for: letter.id) == nil)
    }

    @Test(arguments: [
        SessionRestoration.signedOut, .expired(serverA), .interrupted(serverA), .restoring,
        .unavailable(server: serverA, message: "keychain"), .signedIn(serverB),
    ])
    func signOutOrServerChangeClearsTheActivity(_ next: SessionRestoration) {
        let (tracker, locator) = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.sessionChanged(to: next)

        #expect(tracker.current == nil)
        #expect(locator.vault(for: letter.id) == nil)
    }

    @Test func confirmingTheSameSessionKeepsTheActivity() {
        let (tracker, _) = signedInTracker()
        let letter = summary("Letter")
        tracker.show(letter, vaultID: vault)

        tracker.sessionChanged(to: .signedIn(serverA))

        #expect(tracker.current?.documentID == letter.id)
    }

    @Test func activityIdentifiesTheDocumentWithoutPublishingIt() throws {
        let (tracker, _) = signedInTracker()
        let letter = summary("Household letter")
        tracker.show(letter, vaultID: vault)
        let activity = NSUserActivity(activityType: DocumentActivityPayload.activityType)

        tracker.configure(activity)

        #expect(activity.title == "Household letter")
        #expect(DocumentActivityPayload(userInfo: activity.userInfo) == tracker.current)
        #expect(activity.requiredUserInfoKeys == DocumentActivityPayload.requiredKeys)
        #expect(!activity.isEligibleForPublicIndexing)
        #expect(!activity.isEligibleForSearch)
        #expect(activity.targetContentIdentifier == letter.id.uuidString.lowercased())
        if #available(iOS 26, macOS 26, *) {
            #expect(activity.appEntityIdentifier == EntityIdentifier(for: DocumentEntity.self, identifier: letter.id))
        }
    }

    @Test func clearedActivityCarriesNoDocument() {
        let (tracker, _) = signedInTracker()
        let letter = summary("Household letter")
        tracker.show(letter, vaultID: vault)
        let activity = NSUserActivity(activityType: DocumentActivityPayload.activityType)
        tracker.configure(activity)

        tracker.sessionChanged(to: .signedOut)
        tracker.configure(activity)

        #expect(activity.userInfo?.isEmpty ?? true)
        #expect(activity.title == nil)
        #expect(activity.targetContentIdentifier == nil)
        if #available(iOS 26, macOS 26, *) {
            #expect(activity.appEntityIdentifier == nil)
        }
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
