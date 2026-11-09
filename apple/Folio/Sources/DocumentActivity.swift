import AppIntents
import FolioKit
import SwiftUI

#if os(iOS)
    import UIKit
#endif

extension View {
    /// Publishes the open document as this scene's activity while the view is on screen, and offers its
    /// downloaded preview to transfers so sharing doesn't download it again.
    func documentActivity(_ summary: DocumentSummary, vaultID: UUID, previewFile: URL?) -> some View {
        modifier(DocumentActivityModifier(summary: summary, vaultID: vaultID, previewFile: previewFile))
    }

    /// Share and copy commands for the open document.
    func documentTransferCommands(_ summary: DocumentSummary, vaultID: UUID) -> some View {
        modifier(DocumentTransferCommands(entity: .init(summary, vaultID: vaultID)))
    }

    /// Lets a row be dragged out as its document.
    func draggableDocument(_ document: Document, vaultID: UUID) -> some View {
        draggable(DocumentEntity(DocumentSummary(document, correspondent: nil, documentType: nil, vaultID: vaultID)))
    }

    /// Tells Siri and Apple Intelligence which document each list row shows. Only the Xcode 27 SDK (Swift 6.4)
    /// exposes this modifier to the app target, so builds with an older SDK leave rows without it.
    @ViewBuilder
    func documentEntityIdentifiers(vaultID: UUID) -> some View {
        #if compiler(>=6.4)
            if #available(iOS 18.4, macOS 15.4, *) {
                appEntityIdentifier(forSelectionType: Document.ID.self) { id in
                    EntityIdentifier(
                        for: DocumentEntity.self, identifier: DocumentEntityID(documentID: id, vaultID: vaultID))
                }
            } else {
                self
            }
        #else
            self
        #endif
    }
}

extension DocumentEntity {
    init(_ summary: DocumentSummary, vaultID: UUID) {
        var summary = summary
        summary.vaultID = vaultID
        self.init(summary)
    }
}

private struct DocumentActivityModifier: ViewModifier {
    let summary: DocumentSummary
    let vaultID: UUID
    let previewFile: URL?
    @Environment(DocumentActivityTracker.self) private var tracker: DocumentActivityTracker?

    func body(content: Content) -> some View {
        content
            .userActivity(
                DocumentActivityPayload.activityType, isActive: tracker?.isCurrent(summary.id) ?? false
            ) { activity in
                tracker?.configure(activity)
            }
            .onAppear { tracker?.show(summary, vaultID: vaultID) }
            .onChange(of: summary) { _, summary in tracker?.show(summary, vaultID: vaultID) }
            .onDisappear { tracker?.hide(documentID: summary.id) }
            .onChange(of: previewFile, initial: true) { _, file in
                guard let file else { return }
                TransferFileCache.shared.adopt(file, for: DocumentEntityID(documentID: summary.id, vaultID: vaultID))
            }
    }
}

private struct DocumentTransferCommands: ViewModifier {
    let entity: DocumentEntity

    func body(content: Content) -> some View {
        content
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    ShareLink(item: entity, preview: SharePreview(entity.title))
                        .accessibilityIdentifier("share-document")
                }
                #if os(iOS)
                    ToolbarItem(placement: .secondaryAction) {
                        Button("Copy document", systemImage: "doc.on.doc", action: copy)
                    }
                #endif
            }
            #if os(macOS)
                .copyable([entity])
            #endif
    }

    #if os(iOS)
        private func copy() {
            let provider = NSItemProvider()
            provider.register(entity)
            UIPasteboard.general.setItemProviders([provider], localOnly: false, expirationDate: nil)
        }
    #endif
}
