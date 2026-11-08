import AppIntents
import FolioKit
import SwiftUI

extension View {
    /// Publishes the open document as the scene's activity while this view is on screen.
    func documentActivity(_ summary: DocumentSummary, vaultID: UUID) -> some View {
        modifier(DocumentActivityModifier(summary: summary, vaultID: vaultID))
    }

    /// Tells the system which document a row shows, so requests about on-screen content can name it.
    @ViewBuilder
    func documentEntityIdentifier(_ id: Document.ID) -> some View {
        if #available(iOS 26, macOS 26, *) {
            appEntityIdentifier(EntityIdentifier(for: DocumentEntity.self, identifier: id))
        } else {
            self
        }
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
