import FolioKit
import SwiftUI

extension View {
    /// Publishes the open document as the scene's activity while this view is on screen.
    func documentActivity(_ summary: DocumentSummary, vaultID: UUID) -> some View {
        modifier(DocumentActivityModifier(summary: summary, vaultID: vaultID))
    }
}

private struct DocumentActivityModifier: ViewModifier {
    let summary: DocumentSummary
    let vaultID: UUID
    private let tracker = DocumentActivityTracker.shared

    func body(content: Content) -> some View {
        content
            .userActivity(DocumentActivityPayload.activityType, isActive: tracker.isCurrent(summary.id)) {
                tracker.configure($0)
            }
            .onAppear { tracker.show(summary, vaultID: vaultID) }
            .onChange(of: summary) { _, summary in tracker.show(summary, vaultID: vaultID) }
            .onDisappear { tracker.hide(documentID: summary.id) }
    }
}
