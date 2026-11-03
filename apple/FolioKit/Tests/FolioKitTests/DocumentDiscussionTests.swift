import Foundation
import Testing

@testable import FolioKit

@MainActor
struct DocumentDiscussionTests {
    @Test func savedEntriesRemainVisibleAcrossChronologicalPages() async throws {
        let service = PagedDiscussionStub()
        let model = DocumentDiscussionViewModel(documentID: UUID(), service: service)
        await model.load()
        model.beginAdding()
        model.draft = "Newest"
        await model.save()
        #expect(model.entries.map(\.body) == ["First", "Newest"])
        #expect(model.nextCursor == "next")
        #expect(await service.requestedCursors == [nil])

        await model.loadMore()
        #expect(model.entries.map(\.body) == ["First", "Middle", "Same time", "Newest"])
        #expect(model.nextCursor == "last")
        let middle = try #require(model.entries.first(where: { $0.body == "Middle" }))
        model.beginEditing(middle)
        model.draft = "Updated middle"
        await model.save()
        #expect(model.entries.map(\.body) == ["First", "Updated middle", "Same time", "Newest"])
        let updated = try #require(model.entries.first(where: { $0.id == middle.id }))
        #expect(updated.createdAt == middle.createdAt)
        #expect(model.nextCursor == "last")
        #expect(await service.requestedCursors == [nil, "next"])
    }

    @Test func lostResponseRetainsExactDraftAndKey() async throws {
        let service = DiscussionStub()
        let model = DocumentDiscussionViewModel(documentID: UUID(), service: service)
        await model.load()
        model.beginAdding()
        model.draft = "Keep this note"
        let key = try #require(model.creationKey)
        await model.save()
        #expect(model.draft == "Keep this note")
        #expect(model.creationKey == key)
        #expect(model.error != nil)
        await model.save()
        #expect(await service.keys == [key, key])
        #expect(model.creationKey == nil)
        #expect(model.entries.count == 1)
    }

    @Test func failedEditAndDeleteRetainVisibleContent() async {
        let service = DiscussionStub()
        let model = DocumentDiscussionViewModel(documentID: UUID(), service: service)
        await model.load()
        let entry = model.entries[0]
        model.beginEditing(entry)
        model.draft = "Changed text"
        await model.save()
        #expect(model.isEditing)
        #expect(model.draft == "Changed text")
        #expect(model.entries[0].body == entry.body)
        model.cancel()
        await model.delete(entry)
        #expect(model.entries.count == 1)
        #expect(model.error != nil)
    }

    @Test func revokedAccessClearsPreviouslyLoadedEntries() async {
        let service = DiscussionStub()
        let model = DocumentDiscussionViewModel(documentID: UUID(), service: service)
        await model.load()
        #expect(!model.entries.isEmpty)
        await service.revokeAccess()
        await model.loadMore()
        #expect(model.entries.isEmpty)
        #expect(!model.canCreate)
        #expect(model.nextCursor == nil)
    }

    @Test func paginationDeduplicatesAndViewerCannotStartDraft() async {
        let service = DiscussionStub()
        let model = DocumentDiscussionViewModel(documentID: UUID(), service: service)
        await model.load()
        await model.loadMore()
        #expect(model.entries.count == 1)
        await service.setViewer()
        await model.load()
        model.beginAdding()
        #expect(!model.isEditing)
    }
}

private actor DiscussionStub: DocumentEntryService {
    var keys: [UUID] = []
    var viewer = false
    var revoked = false
    func revokeAccess() { revoked = true }
    let entry = DocumentEntry(
        id: UUID(), documentID: UUID(), vaultID: UUID(), kind: .note,
        body: "Keep this note", createdAt: Date(), updatedAt: Date(), createdBy: "author",
        canEdit: true, canDelete: true)
    func setViewer() { viewer = true }
    func entryPage(documentID: UUID, kind: DocumentEntryKind, cursor: String?) throws -> DocumentEntryPage {
        if revoked { throw FolioError.server(status: 403, message: "Access revoked") }
        return
            DocumentEntryPage(entries: [entry], nextCursor: cursor == nil ? "next" : nil, canCreate: !viewer)
    }
    func createEntry(documentID: UUID, kind: DocumentEntryKind, body: String, key: UUID) throws -> DocumentEntry {
        keys.append(key)
        if keys.count == 1 { throw FolioError.unreachable }
        return entry
    }
}

private actor PagedDiscussionStub: DocumentEntryService {
    private(set) var requestedCursors: [String?] = []
    private let documentID = UUID()
    private let vaultID = UUID()

    private func entry(_ number: Int, body: String, time: TimeInterval) -> DocumentEntry {
        DocumentEntry(
            id: UUID(uuidString: String(format: "00000000-0000-4000-8000-%012d", number))!,
            documentID: documentID, vaultID: vaultID, kind: .note, body: body,
            createdAt: Date(timeIntervalSince1970: time), updatedAt: Date(timeIntervalSince1970: time),
            createdBy: "author", canEdit: true, canDelete: true)
    }

    func entryPage(documentID: UUID, kind: DocumentEntryKind, cursor: String?) -> DocumentEntryPage {
        requestedCursors.append(cursor)
        if cursor == nil {
            return DocumentEntryPage(entries: [entry(1, body: "First", time: 10)], nextCursor: "next", canCreate: true)
        }
        return DocumentEntryPage(
            entries: [
                entry(2, body: "Middle", time: 20), entry(3, body: "Same time", time: 40),
                entry(4, body: "Newest", time: 40),
            ], nextCursor: "last", canCreate: true)
    }

    func createEntry(documentID: UUID, kind: DocumentEntryKind, body: String, key: UUID) -> DocumentEntry {
        entry(4, body: body, time: 40)
    }

    func updateEntry(documentID: UUID, entryID: UUID, body: String) -> DocumentEntry {
        entry(2, body: body, time: 20)
    }
}
