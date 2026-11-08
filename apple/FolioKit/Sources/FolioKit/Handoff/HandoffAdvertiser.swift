import Foundation

/// The list a window shows when no document is selected.
public enum AdvertisedList: Equatable, Sendable {
    case library(HandoffList)
    case savedView(UUID)
}

/// Everything a window's Handoff activity depends on. Search text is deliberately not part of it.
public struct AdvertisingState: Equatable, Sendable {
    public var session: SessionRestoration
    public var isLocked: Bool
    public var vaultID: UUID?
    public var list: AdvertisedList
    public var documentID: Document.ID?
    public var showingDiscussion: Bool
    /// The preview's 1-based page, when it has pages.
    public var page: Int?

    public init(
        session: SessionRestoration, isLocked: Bool, vaultID: UUID?, list: AdvertisedList = .library(.inbox),
        documentID: Document.ID? = nil, showingDiscussion: Bool = false, page: Int? = nil
    ) {
        self.session = session
        self.isLocked = isLocked
        self.vaultID = vaultID
        self.list = list
        self.documentID = documentID
        self.showingDiscussion = showingDiscussion
        self.page = page
    }
}

public enum HandoffAdvertiser {
    /// The one activity a window offers: its selected document, otherwise its library or saved view.
    /// Nothing while locked or not signed in.
    public static func payload(for state: AdvertisingState) -> HandoffPayload? {
        guard !state.isLocked, case let .signedIn(server) = state.session, let vaultID = state.vaultID else {
            return nil
        }
        let target: HandoffTarget
        if let document = state.documentID {
            target = .document(document, pane: state.showingDiscussion ? .discussion : nil, page: state.page)
        } else {
            switch state.list {
            case let .library(list): target = .library(list)
            case let .savedView(id): target = .savedView(id)
            }
        }
        return HandoffPayload(server: server, vaultID: vaultID, target: target)
    }
}
