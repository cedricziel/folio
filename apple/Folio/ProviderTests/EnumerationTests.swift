import FileProvider
import FolioKit
import XCTest

private final class ChangeObserver: NSObject, NSFileProviderChangeObserver {
    var updated: [NSFileProviderItem] = []
    var deleted: [NSFileProviderItemIdentifier] = []
    var error: Error?
    var anchor: NSFileProviderSyncAnchor?
    let finished: XCTestExpectation

    init(_ finished: XCTestExpectation) { self.finished = finished }
    func didUpdate(_ updatedItems: [NSFileProviderItem]) { updated += updatedItems }
    func didDeleteItems(withIdentifiers identifiers: [NSFileProviderItemIdentifier]) { deleted += identifiers }
    func finishEnumeratingChanges(upTo anchor: NSFileProviderSyncAnchor, moreComing: Bool) {
        self.anchor = anchor
        finished.fulfill()
    }
    func finishEnumeratingWithError(_ error: Error) {
        self.error = error
        finished.fulfill()
    }
}

final class EnumerationTests: XCTestCase {
    private var directory: URL!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private func anchor(for enumerator: FolioEnumerator) async throws -> NSFileProviderSyncAnchor {
        let value = await withCheckedContinuation { continuation in
            enumerator.currentSyncAnchor { continuation.resume(returning: $0) }
        }
        return try XCTUnwrap(value)
    }

    private func changes(_ enumerator: FolioEnumerator, from anchor: NSFileProviderSyncAnchor) async -> ChangeObserver {
        let finished = expectation(description: "Changes finish")
        let observer = ChangeObserver(finished)
        enumerator.enumerateChanges(for: observer, from: anchor)
        await fulfillment(of: [finished], timeout: 5)
        return observer
    }

    func testDeletedDocumentIsReportedToFolderObserver() async throws {
        let document = Document(title: "Letter", originalFilename: "letter.pdf")
        let service = MockFolioService(documents: [document])
        let enumerator = FolioEnumerator(catalog: FolioCatalog(service: service), container: .all, directory: directory)
        let anchor = try await anchor(for: enumerator)
        try await service.deleteDocument(id: document.id)
        let observer = await changes(enumerator, from: anchor)

        XCTAssertNil(observer.error)
        XCTAssertEqual(observer.deleted, [FolioItemID.document(document.id, in: .all).providerIdentifier])
    }

    func testWorkingSetReportsEveryDeletedAliasAfterRestart() async throws {
        let tag = Tag(name: "Taxes")
        let document = Document(title: "Letter", originalFilename: "letter.pdf", tags: [tag])
        let service = MockFolioService(documents: [document])
        await service.setTags([tag])
        let catalog = FolioCatalog(service: service)
        let first = FolioEnumerator(catalog: catalog, container: nil, directory: directory)
        let anchor = try await anchor(for: first)
        try await service.deleteDocument(id: document.id)
        let restarted = FolioEnumerator(catalog: catalog, container: nil, directory: directory)
        let observer = await changes(restarted, from: anchor)

        XCTAssertNil(observer.error)
        XCTAssertEqual(
            Set(observer.deleted),
            Set([
                FolioItemID.document(document.id, in: .all).providerIdentifier,
                FolioItemID.document(document.id, in: .inbox).providerIdentifier,
                FolioItemID.document(document.id, in: .tag(tag.id)).providerIdentifier,
            ]))
        XCTAssertTrue(observer.updated.isEmpty)
    }

    func testTaxonomyRenameUpdatesMetadataWithoutDocumentChanges() async throws {
        let tag = Tag(name: "Before")
        let service = MockFolioService()
        await service.setTags([tag])
        let catalog = FolioCatalog(service: service)
        let original = try await catalog.children(of: .tags)[0]
        let enumerator = FolioEnumerator(catalog: catalog, container: .tags, directory: directory)
        let anchor = try await anchor(for: enumerator)
        await service.setTags([Tag(id: tag.id, name: "After")])
        let observer = await changes(enumerator, from: anchor)

        XCTAssertNil(observer.error)
        XCTAssertEqual(observer.updated.map(\.filename), ["After"])
        XCTAssertNotEqual(observer.updated.first?.itemVersion, FolioProviderItem(original).itemVersion)
        XCTAssertNotEqual(observer.anchor, anchor)
        XCTAssertTrue(observer.deleted.isEmpty)
    }

    func testClassificationRemovesInboxAliasAndAddsCorrespondentAlias() async throws {
        let person = Correspondent(name: "Sender")
        var document = Document(title: "Letter", ocrStatus: .done, embedStatus: .done)
        let service = MockFolioService(documents: [document], correspondents: [person])
        let enumerator = FolioEnumerator(catalog: FolioCatalog(service: service), container: nil, directory: directory)
        let anchor = try await anchor(for: enumerator)
        document.correspondentID = person.id
        await service.setDocuments([document])
        let observer = await changes(enumerator, from: anchor)

        XCTAssertNil(observer.error)
        XCTAssertEqual(observer.deleted, [FolioItemID.document(document.id, in: .inbox).providerIdentifier])
        XCTAssertEqual(
            observer.updated.map(\.itemIdentifier),
            [FolioItemID.document(document.id, in: .correspondent(person.id)).providerIdentifier])
    }

    func testUnchangedSnapshotDoesNotReportUpdates() async throws {
        let service = MockFolioService(documents: [Document(title: "Letter")])
        let enumerator = FolioEnumerator(catalog: FolioCatalog(service: service), container: nil, directory: directory)
        let anchor = try await anchor(for: enumerator)
        let observer = await changes(enumerator, from: anchor)
        XCTAssertNil(observer.error)
        XCTAssertEqual(observer.anchor, anchor)
        XCTAssertTrue(observer.updated.isEmpty)
        XCTAssertTrue(observer.deleted.isEmpty)
    }

    func testMissingSnapshotRequestsFullEnumeration() async throws {
        let enumerator = FolioEnumerator(
            catalog: FolioCatalog(service: MockFolioService()), container: .all, directory: directory)
        let observer = await changes(enumerator, from: NSFileProviderSyncAnchor(Data("legacy-anchor".utf8)))
        XCTAssertEqual((observer.error as? NSFileProviderError)?.code, .syncAnchorExpired)
        XCTAssertNil(observer.anchor)
    }

    func testInitialAnchorAndItemsShareOneSnapshot() async throws {
        let document = Document(title: "Letter")
        let service = MockFolioService(documents: [document])
        let state = FolioSyncState(catalog: FolioCatalog(service: service), container: .all, directory: directory)
        let before = try await state.initialSnapshot()
        try await service.deleteDocument(id: document.id)
        let enumeration = try await state.initialSnapshot()
        XCTAssertEqual(enumeration.anchor, before.anchor)
        XCTAssertEqual(enumeration.nodes.map(\.id), [.document(document.id, in: .all)])
        let (_, _, deleted) = try await state.changes(from: before.anchor)
        XCTAssertEqual(deleted, [FolioItemID.document(document.id, in: .all).providerIdentifier])
    }

}
