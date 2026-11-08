import Foundation
import Testing

@testable import FolioKit

private let server = URL(string: "https://folio.example.com/folio")!
private let vault = UUID()

@MainActor struct HandoffActivityTests {
    private func tracker(showing summary: DocumentSummary) -> DocumentActivityTracker {
        let tracker = DocumentActivityTracker(locator: DocumentLocator())
        tracker.sessionChanged(to: .signedIn(server))
        tracker.show(summary, vaultID: vault)
        return tracker
    }

    @Test func aDocumentIsHandedOverWithItsWebPage() throws {
        let summary = DocumentSummary(
            id: UUID(), title: "Tax return 2025", date: Date(timeIntervalSince1970: 0), mimeType: "application/pdf")
        let payload = HandoffPayload(server: server, vaultID: vault, target: .document(summary.id, pane: .discussion))
        let activity = NSUserActivity(activityType: payload.activityType)

        HandoffActivity.configure(activity, payload: payload, tracker: tracker(showing: summary))

        #expect(HandoffPayload(activityType: activity.activityType, userInfo: activity.userInfo) == payload)
        #expect(activity.requiredUserInfoKeys == payload.requiredUserInfoKeys)
        #expect(activity.webpageURL == HandoffLink.webURL(for: payload))
        #expect(activity.isEligibleForHandoff)
        #expect(!activity.isEligibleForSearch)
        #expect(!activity.isEligibleForPublicIndexing)
        #if os(iOS)
            #expect(!activity.isEligibleForPrediction)
        #endif
        // The on-screen activity's identity stays; the title never reaches user info or keywords.
        #expect(activity.title == "Tax return 2025")
        #expect(activity.targetContentIdentifier == summary.id.uuidString.lowercased())
        #expect(!(activity.userInfo ?? [:]).values.contains { "\($0)".contains("Tax return") })
        #expect(activity.keywords.isEmpty)
    }

    @Test func aLibraryCarriesNoTitle() {
        let payload = HandoffPayload(server: server, vaultID: vault, target: .library(.all))
        let activity = NSUserActivity(activityType: payload.activityType)
        activity.title = "Stale"

        HandoffActivity.configure(activity, payload: payload, tracker: DocumentActivityTracker(locator: DocumentLocator()))

        #expect(activity.title == nil)
        #expect(HandoffPayload(activityType: activity.activityType, userInfo: activity.userInfo) == payload)
        #expect(activity.webpageURL?.absoluteString == "https://folio.example.com/folio/vaults/\(vault.uuidString.lowercased())/?view=all")
        #expect(activity.isEligibleForHandoff)
    }

    @Test func withoutAPayloadNothingIsHandedOver() {
        let activity = NSUserActivity(activityType: HandoffActivityType.library)
        HandoffActivity.configure(
            activity, payload: HandoffPayload(server: server, vaultID: vault, target: .library(.inbox)),
            tracker: DocumentActivityTracker(locator: DocumentLocator()))

        HandoffActivity.configure(activity, payload: nil, tracker: DocumentActivityTracker(locator: DocumentLocator()))

        #expect(!activity.isEligibleForHandoff)
        #expect(activity.userInfo?.isEmpty ?? true)
        #expect(activity.webpageURL == nil)
    }
}
