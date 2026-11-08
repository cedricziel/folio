import Foundation
import Testing

@testable import FolioKit

private let server = URL(string: "https://folio.example.com/folio")!
private let vault = UUID()

private func summary(_ title: String) -> DocumentSummary {
    DocumentSummary(id: UUID(), title: title, date: Date(timeIntervalSince1970: 0), mimeType: "application/pdf")
}

@MainActor struct HandoffActivityTests {
    private func tracker(showing summary: DocumentSummary) -> DocumentActivityTracker {
        let tracker = DocumentActivityTracker(isEligibleForHandoff: true)
        tracker.sessionChanged(to: .signedIn(server))
        tracker.show(summary, vaultID: vault)
        return tracker
    }

    @Test func aDocumentIsHandedOverWithItsWebPage() throws {
        let letter = summary("Tax return 2025")
        let payload = HandoffPayload(server: server, vaultID: vault, target: .document(letter.id, pane: .discussion))
        let activity = NSUserActivity(activityType: payload.activityType)

        HandoffActivity.configure(activity, payload: payload, tracker: tracker(showing: letter))

        #expect(HandoffPayload(activityType: activity.activityType, userInfo: activity.userInfo) == payload)
        #expect(activity.requiredUserInfoKeys == payload.requiredUserInfoKeys)
        #expect(activity.webpageURL == HandoffLink.webURL(for: payload))
        #expect(activity.isEligibleForHandoff)
        #expect(!activity.isEligibleForSearch)
        #expect(!activity.isEligibleForPublicIndexing)
        #if os(iOS)
            #expect(!activity.isEligibleForPrediction)
        #endif
        // The on-screen activity's identity stays; the title never reaches the activity.
        #expect(activity.targetContentIdentifier == letter.id.uuidString.lowercased())
        #expect(activity.title == nil)
        #expect(!(activity.userInfo ?? [:]).values.contains { "\($0)".contains("Tax return") })
        #expect(activity.keywords.isEmpty)
    }

    @Test func eachWindowAdvertisesItsOwnDocument() {
        let first = summary("First"), second = summary("Second")
        let firstWindow = tracker(showing: first), secondWindow = tracker(showing: second)
        let firstActivity = NSUserActivity(activityType: HandoffActivityType.document)
        let secondActivity = NSUserActivity(activityType: HandoffActivityType.document)

        HandoffActivity.configure(
            firstActivity, payload: HandoffPayload(server: server, vaultID: vault, target: .document(first.id)),
            tracker: firstWindow)
        HandoffActivity.configure(
            secondActivity, payload: HandoffPayload(server: server, vaultID: vault, target: .document(second.id)),
            tracker: secondWindow)

        #expect(firstWindow.isCurrent(first.id) && !firstWindow.isCurrent(second.id))
        #expect(secondWindow.isCurrent(second.id) && !secondWindow.isCurrent(first.id))
        #expect(firstActivity.targetContentIdentifier == first.id.uuidString.lowercased())
        #expect(secondActivity.targetContentIdentifier == second.id.uuidString.lowercased())
        #expect(
            HandoffPayload(activityType: firstActivity.activityType, userInfo: firstActivity.userInfo)?.target
                == .document(first.id))
        #expect(
            HandoffPayload(activityType: secondActivity.activityType, userInfo: secondActivity.userInfo)?.target
                == .document(second.id))
    }

    @Test func aLibraryCarriesNoTitle() {
        let payload = HandoffPayload(server: server, vaultID: vault, target: .library(.all))
        let activity = NSUserActivity(activityType: payload.activityType)
        activity.title = "Stale"

        HandoffActivity.configure(activity, payload: payload, tracker: nil)

        #expect(activity.title == nil)
        #expect(HandoffPayload(activityType: activity.activityType, userInfo: activity.userInfo) == payload)
        #expect(
            activity.webpageURL?.absoluteString
                == "https://folio.example.com/folio/vaults/\(vault.uuidString.lowercased())/?view=all")
        #expect(activity.isEligibleForHandoff)
    }

    @Test func withoutAPayloadNothingIsHandedOver() {
        let activity = NSUserActivity(activityType: HandoffActivityType.library)
        HandoffActivity.configure(
            activity, payload: HandoffPayload(server: server, vaultID: vault, target: .library(.inbox)), tracker: nil)

        HandoffActivity.configure(activity, payload: nil, tracker: nil)

        #expect(!activity.isEligibleForHandoff)
        #expect(activity.userInfo?.isEmpty ?? true)
        #expect(activity.webpageURL == nil)
    }
}
