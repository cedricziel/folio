import Foundation
import Testing

@testable import FolioKit

/// Apps built with the OS 27 SDK resize freely, so the inbox can switch between
/// list-beside-preview and pushed navigation while a document is open.
@MainActor struct ResizableLayoutTests {
    @Test func narrowingKeepsTheDocumentOpenedBesideTheList() {
        let session = DocumentBrowserSession(service: MockFolioService(), destination: .inbox)
        let id = Document.ID()
        session.selection = id

        session.adaptToLayout(showsPreview: false)

        #expect(session.path == [id])
    }

    @Test func narrowingWithoutASelectionShowsTheList() {
        let session = DocumentBrowserSession(service: MockFolioService(), destination: .inbox)

        session.adaptToLayout(showsPreview: false)

        #expect(session.path.isEmpty)
    }

    @Test func narrowingAfterSelectingAnotherDocumentOpensOnlyThatDocument() {
        let session = DocumentBrowserSession(service: MockFolioService(), destination: .inbox)
        let first = Document.ID(), second = Document.ID()
        session.path = [first]
        session.selection = first

        session.adaptToLayout(showsPreview: true)
        session.selection = second
        session.adaptToLayout(showsPreview: false)

        #expect(session.path == [second])
    }

    @Test func wideningKeepsTheSelection() {
        let session = DocumentBrowserSession(service: MockFolioService(), destination: .inbox)
        let id = Document.ID()
        session.path = [id]
        session.selection = id

        session.adaptToLayout(showsPreview: true)
        session.adaptToLayout(showsPreview: false)

        #expect(session.selection == id)
        #expect(session.path == [id])
    }
}
