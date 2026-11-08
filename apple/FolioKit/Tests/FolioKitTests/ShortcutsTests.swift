import AppIntents
import Foundation
import Testing

@testable import FolioKit

private typealias Tag = FolioKit.Tag

private let utilities = Tag(name: "Utilities")
private let taxes = Tag(name: "Taxes")
private let stadtwerke = Correspondent(name: "Stadtwerke Köln")
private let invoice = DocumentType(name: "Invoice")

private func day(_ offset: Int) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    return calendar.date(byAdding: .day, value: offset, to: Date(timeIntervalSince1970: 1_741_910_400))!
}

private func document(
    _ title: String, day offset: Int = 0, correspondent: Correspondent? = nil, type: DocumentType? = nil,
    tags: [Tag] = []
) -> Document {
    Document(
        title: title, content: "text", originalFilename: "\(title).pdf", mimeType: "application/pdf",
        correspondentID: correspondent?.id, documentTypeID: type?.id, tags: tags, createdAt: day(offset))
}

private func makeCatalog(
    _ documents: [Document] = [], tags: [Tag] = [utilities, taxes]
) async -> (DocumentCatalog, MockFolioService) {
    let service = MockFolioService(documents: documents, correspondents: [stadtwerke], types: [invoice])
    await service.setTags(tags)
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    return (DocumentCatalog(service: service, calendar: calendar), service)
}

struct DocumentCatalogTests {
    @Test func summariesResolveNamesAndTags() async throws {
        let (catalog, _) = await makeCatalog([
            document("Gas bill", correspondent: stadtwerke, type: invoice, tags: [utilities])
        ])
        let found = try await catalog.search("gas")
        let summary = try #require(found.first)
        #expect(summary.correspondent == "Stadtwerke Köln")
        #expect(summary.documentType == "Invoice")
        #expect(summary.tags == ["Utilities"])
    }

    @Test func searchPassesTheQueryAndHonoursTheLimit() async throws {
        let (catalog, service) = await makeCatalog((0..<5).map { document("Doc \($0)", day: $0) })
        let found = try await catalog.search("doc", limit: 2)
        #expect(found.count == 2)
        #expect(await service.queries == ["doc"])
    }

    @Test func latestIsTheNewestDocument() async throws {
        let (catalog, _) = await makeCatalog([
            document("Old", day: 0), document("New", day: 9), document("Middle", day: 4),
        ])
        #expect(try await catalog.latest().title == "New")
        #expect(try await catalog.recent(limit: 2).map(\.title) == ["New", "Middle"])
    }

    @Test func latestWithoutDocumentsSaysSo() async {
        let (catalog, _) = await makeCatalog()
        await #expect(throws: ShortcutFailure.noDocuments) { try await catalog.latest() }
    }

    @Test func findCombinesCorrespondentTypeAndTag() async throws {
        let (catalog, _) = await makeCatalog([
            document("Match", correspondent: stadtwerke, type: invoice, tags: [utilities]),
            document("Wrong tag", correspondent: stadtwerke, type: invoice, tags: [taxes]),
            document("Wrong sender", type: invoice, tags: [utilities]),
        ])
        let found = try await catalog.find(
            DocumentCriteria(
                tagID: utilities.id, correspondentID: stadtwerke.id, documentTypeID: invoice.id))
        #expect(found.map(\.title) == ["Match"])
    }

    @Test func findSendsTheFiltersToTheServer() async throws {
        let (catalog, service) = await makeCatalog([document("A", tags: [utilities])])
        _ = try await catalog.find(
            DocumentCriteria(tagID: utilities.id, correspondentID: stadtwerke.id, from: day(0), through: day(2)))
        let filter = try #require(await service.filters.last)
        #expect(filter.tagIDs == [utilities.id])
        #expect(filter.correspondentID == stadtwerke.id)
        #expect(filter.createdAfter == day(0))
        #expect(filter.createdBefore == day(3))
    }

    @Test func findDateRangeIncludesBothEndDays() async throws {
        let (catalog, _) = await makeCatalog([
            document("Before", day: -1), document("First", day: 0), document("Last", day: 2),
            document("After", day: 3),
        ])
        let found = try await catalog.find(DocumentCriteria(from: day(0), through: day(2)))
        #expect(Set(found.map(\.title)) == ["First", "Last"])
    }

    @Test func documentsForIdsKeepsOnlyKnownOnes() async throws {
        let kept = document("Kept")
        let (catalog, _) = await makeCatalog([kept, document("Other")])
        let found = try await catalog.documents(ids: [kept.id, UUID()])
        #expect(found.map(\.id) == [kept.id])
    }

    @Test func tagLookupMatchesNamesCaseInsensitively() async throws {
        let (catalog, _) = await makeCatalog()
        #expect(try await catalog.tags(matching: "tax").map(\.name) == ["Taxes"])
        #expect(try await catalog.tags(matching: "  ").count == 2)
    }

    @Test func addAndRemoveTagReachTheService() async throws {
        let target = document("Doc")
        let (catalog, service) = await makeCatalog([target])
        try await catalog.addTag(utilities.id, to: target.id)
        try await catalog.removeTag(taxes.id, from: target.id)
        #expect(await service.tagged == [TagAssignment(tag: utilities.id, document: target.id)])
        #expect(await service.untagged == [TagAssignment(tag: taxes.id, document: target.id)])
    }

    @Test func uploadTagsEveryNewDocumentAndTitlesASingleFile() async throws {
        let (catalog, service) = await makeCatalog()
        let file = UploadFile(data: Data([1]), filename: "scan.pdf", mimeType: "application/pdf")
        let one = try await catalog.upload([file], title: " Gas bill ", tagIDs: [utilities.id])
        #expect(one.map(\.title) == ["Gas bill"])
        #expect(await service.tagged.count == 1)

        _ = try await catalog.upload([file, file], title: "Ignored")
        #expect(await service.uploads.suffix(2).allSatisfy { $0.title == nil })
    }

    @Test func fileForUnknownDocumentFails() async {
        let (catalog, _) = await makeCatalog()
        await #expect(throws: ShortcutFailure.documentNotFound) { try await catalog.file(for: UUID()) }
    }
}

private final class IntentRuns: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [TelemetryIntent] = []

    var all: [TelemetryIntent] { lock.withLock { recorded } }

    func append(_ intent: TelemetryIntent) {
        lock.withLock { recorded.append(intent) }
    }
}

@Suite(.serialized, .timeLimit(.minutes(1))) struct IntentTests {
    private let runs = IntentRuns()

    private func install(_ documents: [Document] = []) async -> MockFolioService {
        let (_, service) = await makeCatalog(documents)
        IntentEnvironment.use { service }
        let runs = runs
        IntentEnvironment.recordRuns { runs.append($0) }
        return service
    }

    @Test func everyShortcutRecordsItsRunByNameOnly() async throws {
        let target = document("Gas bill", tags: [utilities])
        _ = await install([target])
        let entity = DocumentEntity(DocumentSummary(target, correspondent: nil, documentType: nil))
        var search = SearchDocumentsIntent(query: "gas")
        search.query = "gas"
        _ = try await search.perform()
        _ = try await GetLatestDocumentIntent().perform()
        _ = try await FindDocumentsIntent().perform()
        _ = try await AddTagIntent(document: entity, tag: TagEntity(taxes)).perform()
        _ = try await RemoveTagIntent(document: entity, tag: TagEntity(utilities)).perform()
        _ = try await GetDocumentFileIntent(document: entity).perform()
        #expect(
            runs.all == [
                .searchDocuments, .getLatestDocument, .findDocuments, .addTag, .removeTag, .getDocumentFile,
            ])
    }

    @Test func searchReturnsEntitiesAndSaysHowMany() async throws {
        _ = await install([document("Gas bill", correspondent: stadtwerke)])
        var intent = SearchDocumentsIntent(query: "gas")
        intent.query = "gas"
        let result = try await intent.perform()
        let documents = try #require(result.value)
        #expect(documents.map(\.title) == ["Gas bill"])
        #expect(documents.first?.correspondent == "Stadtwerke Köln")
    }

    @Test func latestReturnsTheNewestEntity() async throws {
        _ = await install([document("Old", day: 0), document("New", day: 3)])
        let result = try await GetLatestDocumentIntent().perform()
        #expect(result.value?.title == "New")
    }

    @Test func findFiltersByTag() async throws {
        _ = await install([document("A", tags: [utilities]), document("B", tags: [taxes])])
        var intent = FindDocumentsIntent()
        intent.tag = TagEntity(utilities)
        let result = try await intent.perform()
        #expect(result.value?.map(\.title) == ["A"])
    }

    @Test func addTagCallsTheServiceAndConfirms() async throws {
        let target = document("Gas bill")
        let service = await install([target])
        let entity = DocumentEntity(DocumentSummary(target, correspondent: nil, documentType: nil))
        _ = try await AddTagIntent(document: entity, tag: TagEntity(utilities)).perform()
        #expect(await service.tagged == [TagAssignment(tag: utilities.id, document: target.id)])
    }

    @Test func removeTagCallsTheService() async throws {
        let target = document("Gas bill")
        let service = await install([target])
        let entity = DocumentEntity(DocumentSummary(target, correspondent: nil, documentType: nil))
        _ = try await RemoveTagIntent(document: entity, tag: TagEntity(utilities)).perform()
        #expect(await service.untagged == [TagAssignment(tag: utilities.id, document: target.id)])
    }

    @Test func getFileReturnsTheDownloadedFile() async throws {
        let target = document("Gas bill")
        _ = await install([target])
        let entity = DocumentEntity(DocumentSummary(target, correspondent: nil, documentType: nil))
        let result = try await GetDocumentFileIntent(document: entity).perform()
        #expect(result.value?.filename == "Gas bill.pdf")
    }

    @Test func uploadSendsTheFileWithItsType() async throws {
        let service = await install()
        let file = IntentFile(
            data: Data([0x25, 0x50, 0x44, 0x46]), filename: "letter.pdf", type: .pdf)
        _ = try await UploadFileIntent(files: [file], title: "Letter", tags: [TagEntity(utilities)]).perform()
        let sent = try #require(await service.uploads.first)
        #expect(sent.filename == "letter.pdf")
        #expect(sent.mimeType == "application/pdf")
        #expect(sent.title == "Letter")
        #expect(await service.tagged.count == 1)
    }

    @Test func uploadRejectsAnEmptyFile() async {
        _ = await install()
        let empty = IntentFile(data: Data(), filename: "empty.pdf", type: .pdf)
        await #expect(throws: ShortcutError.self) {
            try await UploadFileIntent(files: [empty]).perform()
        }
    }

    @MainActor @Test func scanAndOpenLeaveARequestForTheApp() async throws {
        let router = IntentRouter.shared
        _ = try await ScanDocumentIntent().perform()
        #expect(router.request == .scan)

        let target = document("Gas bill")
        let entity = DocumentEntity(DocumentSummary(target, correspondent: nil, documentType: nil))
        _ = try await OpenDocumentIntent(target: entity).perform()
        #expect(router.request == .document(target.id))
        router.clear()
    }

    @Test func entityQueriesUseTheService() async throws {
        let target = document("Gas bill", correspondent: stadtwerke, type: invoice)
        _ = await install([target])
        #expect(try await DocumentEntityQuery().entities(for: [target.id]).map(\.title) == ["Gas bill"])
        #expect(try await DocumentEntityQuery().entities(matching: "gas").count == 1)
        #expect(try await TagEntityQuery().entities(matching: "util").map(\.name) == ["Utilities"])
        #expect(try await CorrespondentEntityQuery().suggestedEntities().map(\.name) == ["Stadtwerke Köln"])
        #expect(try await DocumentTypeEntityQuery().entities(for: [invoice.id]).map(\.name) == ["Invoice"])
        #expect(try await VaultEntityQuery().suggestedEntities().isEmpty)
    }

    @Test func notSignedInSaysToOpenTheApp() async {
        IntentEnvironment.use { nil }
        await #expect(throws: ShortcutError.notSignedIn) {
            try await GetLatestDocumentIntent().perform()
        }
    }
}

struct ShortcutErrorTests {
    @Test(arguments: [
        (FolioError.unreachable as any Error, "Couldn't reach your Folio server. Check the connection and try again."),
        (FolioError.signInRequired, ShortcutError.notSignedIn.message),
        (FolioError.server(status: 500, message: nil), "The server answered with status 500."),
        (FolioError.server(status: 400, message: "File too large"), "File too large"),
        (ShortcutFailure.noDocuments, "There are no documents in Folio yet."),
        (ShortcutFailure.unreadableFile("a.pdf"), "Couldn't read a.pdf."),
    ])
    func mapsToPlainSentences(error: any Error, message: String) {
        #expect(ShortcutError(mapping: error).message == message)
    }

    @Test func messagesFollowTheVoice() {
        let messages = [ShortcutError.notSignedIn.message, ShortcutError(mapping: FolioError.unreachable).message]
        #expect(messages.allSatisfy { !$0.contains("!") && !$0.lowercased().contains("oops") })
    }
}
