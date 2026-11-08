import FolioKit
import SwiftUI

/// What a window's Handoff activity needs from above the workspace: the session and the lock.
struct HandoffContext: Equatable {
    var session = SessionRestoration.signedOut
    var isLocked = false
}

extension EnvironmentValues {
    @Entry var handoffContext = HandoffContext()
}

extension View {
    /// Publishes the open document as the scene's activity while this view is on screen, for the
    /// system's on-screen awareness and for Handoff.
    func documentActivity(
        _ summary: DocumentSummary, vaultID: UUID, showingDiscussion: Bool = false, page: Int? = nil
    ) -> some View {
        modifier(
            DocumentActivityModifier(summary: summary, vaultID: vaultID, showingDiscussion: showingDiscussion, page: page)
        )
    }

    /// Offers the list on screen for Handoff while no document is selected.
    func listActivity(vaultID: UUID, list: AdvertisedList, selection: Document.ID?) -> some View {
        modifier(ListActivityModifier(vaultID: vaultID, list: list, selection: selection))
    }
}

private struct DocumentActivityModifier: ViewModifier {
    let summary: DocumentSummary
    let vaultID: UUID
    let showingDiscussion: Bool
    let page: Int?
    @Environment(\.handoffContext) private var context
    private let tracker = DocumentActivityTracker.shared

    private var payload: HandoffPayload? {
        HandoffAdvertiser.payload(
            for: AdvertisingState(
                session: context.session, isLocked: context.isLocked, vaultID: vaultID, documentID: summary.id,
                showingDiscussion: showingDiscussion, page: page))
    }

    func body(content: Content) -> some View {
        let payload = payload
        content
            .userActivity(DocumentActivityPayload.activityType, isActive: tracker.isCurrent(summary.id) && payload != nil)
        {
            HandoffActivity.configure($0, payload: payload, tracker: tracker)
        }
        .onAppear { tracker.show(summary, vaultID: vaultID) }
        .onChange(of: summary) { _, summary in tracker.show(summary, vaultID: vaultID) }
        .onDisappear { tracker.hide(documentID: summary.id) }
    }
}

/// One activity per list type, so a window switching between a library and a saved view never
/// changes an activity's type in place.
private struct ListActivityModifier: ViewModifier {
    let vaultID: UUID
    let list: AdvertisedList
    let selection: Document.ID?
    @Environment(\.handoffContext) private var context

    private var payload: HandoffPayload? {
        HandoffAdvertiser.listPayload(
            for: AdvertisingState(
                session: context.session, isLocked: context.isLocked, vaultID: vaultID, list: list,
                documentID: selection))
    }

    func body(content: Content) -> some View {
        let payload = payload
        let library = payload?.activityType == HandoffActivityType.library ? payload : nil
        let savedView = payload?.activityType == HandoffActivityType.savedView ? payload : nil
        content
            .userActivity(HandoffActivityType.library, isActive: library != nil) {
                HandoffActivity.configure($0, payload: library)
            }
            .userActivity(HandoffActivityType.savedView, isActive: savedView != nil) {
                HandoffActivity.configure($0, payload: savedView)
            }
    }
}
