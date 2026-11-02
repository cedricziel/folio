import Foundation
import Observation

/// Library destinations currently backed by the document service.
public enum WorkspaceDestination: String, CaseIterable, Sendable {
    case inbox
    case all

    public var title: String {
        switch self {
        case .inbox: "Inbox"
        case .all: "All documents"
        }
    }

    public var systemImage: String {
        switch self {
        case .inbox: "tray"
        case .all: "doc.on.doc"
        }
    }
}

/// Browser state retained while another destination is visible.
@MainActor @Observable
public final class DocumentBrowserSession {
    public let list: DocumentListViewModel
    public var selection: Document.ID?
    public var path: [Document.ID] = []

    init(service: any FolioService, destination: WorkspaceDestination) {
        list = DocumentListViewModel(service: service)
        list.scope = destination == .inbox ? .inbox : .all
    }
}

/// In-memory navigation for one signed-in workspace; discard it on account changes.
@MainActor @Observable
public final class WorkspaceViewModel {
    public var destination = WorkspaceDestination.inbox
    private let inbox: DocumentBrowserSession
    private let all: DocumentBrowserSession

    public init(service: any FolioService) {
        inbox = DocumentBrowserSession(service: service, destination: .inbox)
        all = DocumentBrowserSession(service: service, destination: .all)
    }

    public var session: DocumentBrowserSession {
        destination == .inbox ? inbox : all
    }

    /// Open an external document link in the unfiltered library.
    public func open(_ id: Document.ID) {
        destination = .all
        all.list.searchText = ""
        all.selection = id
        all.path = [id]
    }
}
