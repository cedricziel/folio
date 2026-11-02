import Foundation
import Testing

@testable import FolioKit

@MainActor struct WorkspaceTests {
    @Test func firstSessionOpensInbox() {
        let workspace = WorkspaceViewModel(service: MockFolioService())
        #expect(workspace.destination == .inbox)
        #expect(workspace.session.list.scope == .inbox)
    }

    @Test func switchingDestinationsPreservesEachBrowser() {
        let workspace = WorkspaceViewModel(service: MockFolioService())
        let documentID = UUID()
        workspace.session.list.searchText = "electricity"
        workspace.session.selection = documentID
        workspace.session.path = [documentID]

        workspace.destination = .all
        #expect(workspace.session.list.scope == .all)
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.path.isEmpty)
        workspace.session.list.searchText = "insurance"

        workspace.destination = .inbox
        #expect(workspace.session.list.searchText == "electricity")
        #expect(workspace.session.selection == documentID)
        #expect(workspace.session.path == [documentID])
        workspace.session.path = []
        #expect(workspace.session.selection == documentID)

        workspace.destination = .all
        #expect(workspace.session.list.searchText == "insurance")
    }

    @Test func openingDocumentUsesUnfilteredLibraryWithoutErasingInbox() {
        let workspace = WorkspaceViewModel(service: MockFolioService())
        workspace.session.list.searchText = "inbox search"
        let documentID = UUID()
        workspace.open(documentID)
        #expect(workspace.destination == .all)
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.path == [documentID])
        workspace.destination = .inbox
        #expect(workspace.session.list.searchText == "inbox search")
    }
}
