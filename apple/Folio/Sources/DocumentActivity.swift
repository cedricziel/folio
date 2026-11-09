import AppIntents
import FolioKit
import SwiftUI

#if os(iOS)
    import UIKit
#endif

/// What a window's Handoff activity needs from above the workspace: the session and the lock.
struct HandoffContext: Equatable {
    var session = SessionRestoration.signedOut
    var isLocked = false
}

extension EnvironmentValues {
    @Entry var handoffContext = HandoffContext()
}

extension View {
    /// Publishes the open document as this scene's activity while the view is on screen, for the system's
    /// on-screen awareness and for Handoff, and offers its downloaded preview to transfers so sharing
    /// doesn't download it again.
    func documentActivity(
        _ summary: DocumentSummary, vaultID: UUID, previewFile: URL?, showingDiscussion: Bool = false,
        page: Int? = nil
    ) -> some View {
        modifier(
            DocumentActivityModifier(
                summary: summary, vaultID: vaultID, previewFile: previewFile, showingDiscussion: showingDiscussion,
                page: page))
    }

    /// Offers the list on screen for Handoff while no document is selected.
    func listActivity(vaultID: UUID, list: AdvertisedList, selection: Document.ID?) -> some View {
        modifier(ListActivityModifier(vaultID: vaultID, list: list, selection: selection))
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
    let showingDiscussion: Bool
    let page: Int?
    @Environment(DocumentActivityTracker.self) private var tracker: DocumentActivityTracker?
    @Environment(\.handoffContext) private var context

    /// The Handoff extras for this window's document; nil while locked or signed out.
    private var payload: HandoffPayload? {
        HandoffAdvertiser.payload(
            for: AdvertisingState(
                session: context.session, isLocked: context.isLocked, vaultID: vaultID, documentID: summary.id,
                showingDiscussion: showingDiscussion, page: page))
    }

    func body(content: Content) -> some View {
        let payload = payload
        content
            .userActivity(
                DocumentActivityPayload.activityType,
                isActive: (tracker?.isCurrent(summary.id) ?? false) && payload != nil
            ) { activity in
                HandoffActivity.configure(activity, payload: payload, tracker: tracker)
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

/// The library or saved view on screen, offered for Handoff while no document is selected.
///
/// One `.userActivity` per type, and at most one of them active: an activity's type is fixed when SwiftUI
/// creates it, so switching between a library and a saved view must hand over to the other modifier rather
/// than change a type in place.
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
                HandoffActivity.configure($0, payload: library, tracker: nil)
            }
            .userActivity(HandoffActivityType.savedView, isActive: savedView != nil) {
                HandoffActivity.configure($0, payload: savedView, tracker: nil)
            }
    }
}
