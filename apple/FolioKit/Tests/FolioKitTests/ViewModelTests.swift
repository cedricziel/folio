import Foundation
import Testing

@testable import FolioKit

private let stadtwerke = Correspondent(name: "Stadtwerke Köln")
private let invoice = DocumentType(name: "Invoice")

private func document(
    _ title: String, content: String? = "text", correspondent: Correspondent? = nil,
    type: DocumentType? = nil, pages: Int? = 3, ocr: Document.ProcessingStatus = .done
) -> Document {
    Document(
        title: title, content: content, correspondentID: correspondent?.id,
        documentTypeID: type?.id, pageCount: pages,
        ocrStatus: ocr, embedStatus: .skipped,
        createdAt: Date(timeIntervalSince1970: 1_741_950_000)
    )
}

struct DocumentStatusTests {
    @Test func pendingOcrIsStillBeingRead() {
        #expect(document("a", content: nil, pages: 3, ocr: .pending).status == .reading(pages: 3))
        #expect(document("a", content: "text", pages: nil, ocr: .running).status == .reading(pages: nil))
    }

    @Test func readDocumentWithoutCorrespondentOrTypeNeedsAGlance() {
        #expect(document("a").status == .needsReview)
    }

    @Test func skippedOcrWithoutTextNeedsReview() {
        #expect(document("a", content: nil, ocr: .skipped).status == .needsReview)
    }

    @Test func documentWithAnyClassificationIsFiled() {
        #expect(document("a", correspondent: stadtwerke).status == .filed)
        #expect(document("a", type: invoice).status == .filed)
    }

    @Test func tagTintIsStablePerName() {
        #expect(TagTint.tint(forTagNamed: "Insurance") == TagTint.tint(forTagNamed: "insurance"))
    }
}

@MainActor struct DocumentListViewModelTests {
    private func makeModel(
        _ service: MockFolioService, debounce: Duration = .milliseconds(1)
    ) -> DocumentListViewModel {
        DocumentListViewModel(service: service, searchDebounce: debounce)
    }

    @Test func inboxScopeKeepsOnlyDocumentsWaitingForALook() async {
        let service = MockFolioService(
            documents: [
                document("Unfiled"), document("Filed", correspondent: stadtwerke),
                document("Reading", content: nil, ocr: .pending),
            ], correspondents: [stadtwerke]
        )
        let model = makeModel(service)
        await model.load()
        #expect(model.rows.count == 3)

        model.scope = .inbox
        #expect(model.rows.map(\.title) == ["Unfiled", "Reading"])

        model.scope = .all
        #expect(model.rows.count == 3)
    }

    @Test func loadFillsRowsWithCorrespondentTypeAndStatus() async {
        let service = MockFolioService(
            documents: [
                document("Jahresabrechnung", correspondent: stadtwerke, type: invoice),
                document("Scan", content: nil, pages: 3, ocr: .pending),
                document("Letter"),
            ],
            correspondents: [stadtwerke], types: [invoice]
        )
        let model = makeModel(service)
        await model.load()

        #expect(model.phase == .loaded)
        let rows = model.rows
        #expect(rows[0].subtitle == "Stadtwerke Köln · Invoice")
        #expect(rows[0].statusText == nil)
        #expect(rows[1].statusText == "Reading 3 pages…")
        #expect(rows[2].statusText == "Needs a look")
        #expect(rows[2].subtitle == nil)
        #expect(rows[0].dateText.count == 10)
    }

    @Test func singlePageReadingUsesSingular() async {
        let model = makeModel(MockFolioService(documents: [document("x", content: nil, pages: 1, ocr: .pending)]))
        await model.load()
        #expect(model.rows[0].statusText == "Reading 1 page…")
    }

    @Test func failedLoadKeepsAFriendlyMessage() async {
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        let model = makeModel(service)
        await model.load()
        #expect(model.phase == .failed("Couldn't reach the server."))
    }

    @Test func failedRefreshKeepsRowsForTheSameQuery() async {
        let service = MockFolioService(documents: [document("Letter")])
        let model = makeModel(service)
        await model.load()
        await service.setFailure(.unreachable)
        await model.load()
        #expect(model.rows.count == 1)
    }

    @Test func failedSearchDoesNotPassOffEarlierRowsAsResults() async {
        let service = MockFolioService(documents: [document("Letter")])
        let model = makeModel(service)
        await model.load()
        await service.setFailure(.unreachable)
        model.searchText = "invoice"
        await model.load()
        #expect(model.rows.isEmpty)
    }

    @Test func lookupFailureDoesNotBlockTheList() async {
        let service = MockFolioService(documents: [document("Letter")])
        await service.setFailLookups(true)
        let model = makeModel(service)
        await model.load()
        #expect(model.phase == .loaded)
        #expect(model.rows.count == 1)
    }

    @Test func searchQueriesTheServerAfterDebounce() async {
        let service = MockFolioService(documents: [document("Strom")])
        let model = makeModel(service)
        await model.search("  strom ")
        #expect(await service.queries == ["strom"])
        #expect(model.rows.count == 1)
    }

    @Test func supersededSearchNeverReachesTheServer() async {
        let service = MockFolioService(documents: [document("Strom")])
        let model = makeModel(service, debounce: .milliseconds(200))
        let first = Task { await model.search("st") }
        try? await Task.sleep(for: .milliseconds(20))
        first.cancel()
        await model.search("strom")
        await first.value
        #expect(await service.queries == ["strom"])
    }

    @Test func uploadReloadOnlyAnimatesWhenNewRowsArrive() async {
        let kept = document("Keep")
        let added = document("Uploaded")
        let service = MockFolioService(documents: [kept])
        let model = makeModel(service)
        await model.load()
        let revision = model.rowChangeRevision

        await service.setDocuments([added, kept])
        await model.load(afterUpload: true)

        #expect(model.rows.map(\.id) == [added.id, kept.id])
        #expect(model.rowChangeRevision == revision + 1)
        await model.load(afterUpload: true)
        #expect(model.rowChangeRevision == revision + 1)
    }

    @Test func ordinaryReloadDoesNotAnimateNewRows() async {
        let service = MockFolioService(documents: [document("Keep")])
        let model = makeModel(service)
        await model.load()
        let revision = model.rowChangeRevision

        await service.setDocuments([document("New")])
        await model.load()
        await model.search("new")

        #expect(model.rowChangeRevision == revision)
    }

    @Test func deleteRemovesTheRowAfterTheServerConfirms() async {
        let kept = document("Keep")
        let gone = document("Gone")
        let service = MockFolioService(documents: [kept, gone])
        let model = makeModel(service)
        await model.load()
        let revision = model.rowChangeRevision
        await model.delete(gone)
        #expect(model.rowChangeRevision == revision + 1)
        #expect(model.rows.map(\.title) == ["Keep"])
        #expect(await service.deleted == [gone.id])
    }
}

@MainActor struct DocumentListFailureTests {
    @Test func failedDeleteKeepsTheListAndReportsTransiently() async {
        let doc = Document(title: "Keep", content: "text")
        let service = MockFolioService(documents: [doc])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        await model.load()
        await service.setFailure(.unreachable)
        let revision = model.rowChangeRevision
        await model.delete(doc)
        #expect(model.rowChangeRevision == revision)
        #expect(model.phase == .loaded)
        #expect(model.rows.count == 1)
        #expect(model.actionError == "Couldn't reach the server.")
        model.dismissActionError()
        #expect(model.actionError == nil)
    }

    @Test func lookupsAreRetriedAfterAFailure() async {
        let person = Correspondent(name: "Stadtwerke")
        let service = MockFolioService(documents: [Document(title: "A")], correspondents: [person])
        await service.setFailLookups(true)
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        await model.search("")
        #expect(model.correspondents.isEmpty)
        await service.setFailLookups(false)
        await model.search("")
        #expect(model.correspondents[person.id] == "Stadtwerke")
    }

    @Test func lookupsKeepTrashedNamesAndReloadAfterChanges() async throws {
        let person = Correspondent(name: "Stadtwerke")
        let trashedID = UUID()
        let trashed = try JSONDecoder.taxonomy.decode(
            TaxonomyEntry.self,
            from: Data(
                """
                {"id":"\(trashedID)","vault_id":"\(UUID())","name":"Old landlord",
                "created_at":"2025-03-14T10:15:30Z","updated_at":"2025-03-15T10:15:30Z",
                "deleted_at":"2025-03-15T10:15:30Z"}
                """.utf8))
        let service = MockFolioService(documents: [Document(title: "A")], correspondents: [person])
        await service.setTrashedTaxonomy([trashed], kind: .correspondents)
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        await model.search("")
        #expect(model.correspondents[trashedID] == "Old landlord (deleted)")
        await service.setCorrespondents([Correspondent(id: person.id, name: "City utilities")])
        await model.reloadLookups()
        #expect(model.correspondents[person.id] == "City utilities")
    }
}

extension JSONDecoder {
    static var taxonomy: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}

@MainActor struct DocumentDetailViewModelTests {
    @Test func historyShowsImmutableCreatorSystemUpdatesAndDeletion() {
        let model = DocumentDetailViewModel(
            document: Document(
                title: "Retained history", createdBy: "archive-creator", updatedBy: nil,
                deletedAt: Date(timeIntervalSince1970: 1_741_950_000)),
            service: MockFolioService()
        )
        let history = Dictionary(uniqueKeysWithValues: model.history.map { ($0.label, $0) })
        #expect(history["Created by"]?.value == "archive-creator")
        #expect(history["Updated by"]?.value == "System or unknown")
        #expect(history["Created"]?.isRecord == true)
        #expect(history["Updated"]?.isRecord == true)
        #expect(history["Deleted"]?.isRecord == true)
    }

    @Test func reopeningDetailsRefreshesTheSavedDocumentDate() async throws {
        let service = MockFolioService()
        let original = Document(title: "Letter")
        var saved = original
        saved.documentDate = try #require(DocumentDate(rawValue: "2026-11-02"))
        await service.setDocuments([saved])
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.refreshMetadata()
        #expect(model.document.documentDate == saved.documentDate)
        #expect(model.metadataFailure == nil)
    }

    @Test func aSlowRefreshDoesNotRevertASavedDocumentDate() async throws {
        let service = MockFolioService()
        let original = Document(title: "Letter", canEditDocumentDate: true)
        await service.setDocuments([original])
        await service.suspendDocumentReads()
        let model = DocumentDetailViewModel(document: original, service: service)
        let refresh = Task { await model.refreshMetadata() }
        await service.waitForDocumentRead()
        let date = try #require(DocumentDate(rawValue: "2024-01-05"))
        try await model.saveDocumentDate(date)
        await service.releaseDocumentReads()
        await refresh.value
        #expect(model.document.documentDate == date)
        #expect(model.metadata.first { $0.label == "Document date" }?.value == "2024-01-05")
    }

    @Test func documentDateSaveAndFailurePreserveImportTime() async throws {
        let service = MockFolioService()
        let original = Document(title: "Letter", canEditDocumentDate: true, createdAt: Date(timeIntervalSince1970: 0))
        await service.setDocuments([original])
        let model = DocumentDetailViewModel(document: original, service: service)
        let date = try #require(DocumentDate(rawValue: "1582-10-10"))
        try await model.saveDocumentDate(date)
        #expect(model.document.documentDate == date)
        #expect(model.document.createdAt == original.createdAt)
        await service.setFailure(.unreachable)
        await #expect(throws: FolioError.self) { try await model.saveDocumentDate(nil) }
        #expect(model.document.documentDate == date)
        await service.setFailure(nil)
        try await model.saveDocumentDate(nil)
        #expect(model.document.documentDate == nil)
    }

    @Test func metadataListsRecordsInMono() {
        let model = DocumentDetailViewModel(
            document: Document(
                title: "Bill", originalFilename: "bill.pdf", correspondentID: stadtwerke.id,
                documentTypeID: invoice.id, pageCount: 2, createdAt: Date(timeIntervalSince1970: 1_741_950_000)
            ),
            lookups: lookups, service: MockFolioService()
        )
        let byLabel = Dictionary(uniqueKeysWithValues: model.metadata.map { ($0.label, $0) })
        #expect(byLabel["From"]?.value == "Stadtwerke Köln")
        #expect(byLabel["From"]?.isRecord == false)
        #expect(byLabel["Pages"]?.value == "2")
        #expect(byLabel["Pages"]?.isRecord == true)
        #expect(byLabel["File"]?.value == "bill.pdf")
    }

    @Test func missingClassificationReadsAsNotSet() {
        let model = DocumentDetailViewModel(
            document: Document(title: "Bill"), service: MockFolioService()
        )
        #expect(model.metadata.first { $0.label == "From" }?.value == "Not set")
    }

    @Test func previewLoadsTheDownloadedFile() async {
        let doc = Document(title: "Bill", originalFilename: "bill.pdf")
        let model = DocumentDetailViewModel(
            document: doc, service: MockFolioService()
        )
        await model.loadPreview()
        #expect(model.preview == .ready(URL(filePath: "/tmp/bill.pdf")))
    }

    @Test func previewFailureIsReportedInWords() async {
        let service = MockFolioService()
        await service.setFailure(.server(status: 404, message: "Document has no file"))
        let model = DocumentDetailViewModel(
            document: Document(title: "Bill"), service: service
        )
        await model.loadPreview()
        #expect(model.preview == .failed("Document has no file"))
    }

    @Test func cancelledNavigationTaskStillLoadsTheNextPreview() async {
        let service = MockFolioService()
        let first = DocumentDetailViewModel(
            document: Document(title: "First", originalFilename: "first.pdf"),
            service: service
        )
        await first.loadPreview()
        #expect(first.preview == .ready(URL(filePath: "/tmp/first.pdf")))

        let next = DocumentDetailViewModel(
            document: Document(title: "Next", originalFilename: "next.png"),
            service: service
        )
        let navigation = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            await next.loadPreview()
        }
        await navigation.value
        #expect(next.preview == .ready(URL(filePath: "/tmp/next.png")))
    }

    @Test func reappearingPreviewKeepsTheDownloadedFile() async {
        let service = MockFolioService()
        let model = DocumentDetailViewModel(
            document: Document(title: "Bill", originalFilename: "bill.pdf"),
            service: service
        )
        await model.loadPreview()
        await service.setFailure(.unreachable)
        await model.loadPreview()
        #expect(model.preview == .ready(URL(filePath: "/tmp/bill.pdf")))
    }

    @Test func failedPreviewCanRetryWithoutRestartingTheApp() async {
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        let model = DocumentDetailViewModel(
            document: Document(title: "Bill", originalFilename: "bill.pdf"),
            service: service
        )
        await model.loadPreview()
        #expect(model.preview == .failed("Couldn't reach the server."))
        await service.setFailure(nil)
        await model.loadPreview()
        #expect(model.preview == .ready(URL(filePath: "/tmp/bill.pdf")))
    }

    @Test func refreshedListDataUpdatesTheSameDocumentAndKeepsThePreview() async {
        let service = MockFolioService()
        let original = Document(
            title: "Scan 0001", originalFilename: "scan.pdf", updatedAt: Date(timeIntervalSince1970: 100),
            updatedBy: "uploader")
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.loadPreview()

        var processed = original
        processed.title = "Electricity bill"
        processed.correspondentID = stadtwerke.id
        processed.documentTypeID = invoice.id
        processed.pageCount = 3
        processed.tags = [Tag(name: "Utilities")]
        processed.updatedAt = Date(timeIntervalSince1970: 200)
        processed.updatedBy = "classifier"
        model.update(document: processed, lookups: lookups)
        await model.loadPreview()

        let byLabel = Dictionary(uniqueKeysWithValues: model.metadata.map { ($0.label, $0.value) })
        let history = Dictionary(uniqueKeysWithValues: model.history.map { ($0.label, $0.value) })
        #expect(model.document.title == "Electricity bill")
        #expect(model.document.tags.map(\.name) == ["Utilities"])
        #expect(byLabel["From"] == "Stadtwerke Köln")
        #expect(byLabel["Type"] == "Invoice")
        #expect(byLabel["Pages"] == "3")
        #expect(history["Updated by"] == "classifier")
        #expect(model.summary.title == "Electricity bill")
        #expect(model.summary.correspondent == "Stadtwerke Köln")
        #expect(model.summary.documentType == "Invoice")
        #expect(model.summary.tags == ["Utilities"])
        #expect(model.summary.pageCount == 3)
        #expect(model.preview == .ready(URL(filePath: "/tmp/scan.pdf")))
        #expect(await service.downloads == 1)
    }

    @Test func aNewerSearchRowWithoutTagsKeepsTheShownTags() async {
        let service = MockFolioService()
        let original = Document(
            title: "Scan 0001", tags: [Tag(name: "Utilities")], updatedAt: Date(timeIntervalSince1970: 100))
        let model = DocumentDetailViewModel(document: original, service: service)
        var row = original
        row.title = "Electricity bill"
        row.tags = []
        row.updatedAt = Date(timeIntervalSince1970: 200)
        await service.setFailure(.unreachable)

        model.update(document: row, lookups: lookups)
        await model.refreshMetadata()

        #expect(model.document.title == "Electricity bill")
        #expect(model.document.tags.map(\.name) == ["Utilities"])
        #expect(model.summary.tags == ["Utilities"])
    }

    @Test func anOlderListSnapshotDoesNotRevertFetchedDetails() async throws {
        let service = MockFolioService()
        let stale = Document(
            title: "Scan 0001", correspondentID: stadtwerke.id, updatedAt: Date(timeIntervalSince1970: 100))
        var current = stale
        current.title = "Electricity bill"
        current.updatedAt = Date(timeIntervalSince1970: 200)
        await service.setDocuments([current])
        let model = DocumentDetailViewModel(document: stale, service: service)
        await model.refreshMetadata()

        model.update(document: stale, lookups: lookups)

        #expect(model.document.title == "Electricity bill")
        #expect(model.summary.title == "Electricity bill")
        #expect(model.metadata.first { $0.label == "From" }?.value == "Stadtwerke Köln")
    }

    @Test func aSlowRefreshDoesNotRevertNewerListData() async {
        let service = MockFolioService()
        let original = Document(title: "Scan 0001", updatedAt: Date(timeIntervalSince1970: 100))
        await service.setDocuments([original])
        await service.suspendDocumentReads()
        let model = DocumentDetailViewModel(document: original, service: service)
        let refresh = Task { await model.refreshMetadata() }
        await service.waitForDocumentRead()
        var row = original
        row.title = "Electricity bill"
        row.updatedAt = Date(timeIntervalSince1970: 200)

        model.update(document: row, lookups: lookups)
        await service.releaseDocumentReads()
        await refresh.value

        #expect(model.document.title == "Electricity bill")
        #expect(model.summary.title == "Electricity bill")
    }

    @Test func aRefreshStartedBeforeAReviewDoesNotUndoIt() async throws {
        let service = MockFolioService()
        let original = Document(title: "Letter", reviewed: false, canReviewDocument: true)
        await service.setDocuments([original])
        await service.suspendDocumentReads()
        let model = DocumentDetailViewModel(document: original, service: service)
        let refresh = Task { await model.refreshMetadata() }
        await service.waitForDocumentRead()

        try await model.saveReview(true)
        await service.releaseDocumentReads()
        await refresh.value

        #expect(model.document.reviewed == true)
    }

    @Test func aCancelledRefreshChangesNothing() async {
        let service = MockFolioService()
        let original = Document(title: "Scan 0001", updatedAt: Date(timeIntervalSince1970: 100))
        var renamed = original
        renamed.title = "Electricity bill"
        renamed.updatedAt = Date(timeIntervalSince1970: 200)
        await service.setDocuments([renamed])
        await service.suspendDocumentReads()
        let model = DocumentDetailViewModel(document: original, service: service)
        let refresh = Task { await model.refreshMetadata() }
        await service.waitForDocumentRead()

        refresh.cancel()
        await service.releaseDocumentReads()
        await refresh.value

        #expect(model.document.title == "Scan 0001")
        #expect(model.metadataFailure == nil)
    }

    @Test func fetchedDetailsNameTheFetchedCorrespondent() async {
        let service = MockFolioService()
        let gasag = Correspondent(name: "GASAG")
        let original = Document(
            title: "Bill", correspondentID: stadtwerke.id, updatedAt: Date(timeIntervalSince1970: 100))
        var moved = original
        moved.correspondentID = gasag.id
        moved.updatedAt = Date(timeIntervalSince1970: 200)
        await service.setDocuments([moved])
        var names = lookups
        names.correspondents[gasag.id] = gasag.name
        let model = DocumentDetailViewModel(document: original, lookups: names, service: service)

        await model.refreshMetadata()

        #expect(model.metadata.first { $0.label == "From" }?.value == "GASAG")
        #expect(model.summary.correspondent == "GASAG")
    }

    @Test func fetchedDetailsRefreshEveryField() async {
        let service = MockFolioService()
        let original = Document(title: "Scan 0001", updatedAt: Date(timeIntervalSince1970: 100))
        var processed = original
        processed.title = "Electricity bill"
        processed.pageCount = 2
        processed.updatedAt = Date(timeIntervalSince1970: 200)
        await service.setDocuments([processed])
        let model = DocumentDetailViewModel(document: original, service: service)

        await model.refreshMetadata()

        #expect(model.metadata.first { $0.label == "Pages" }?.value == "2")
        #expect(model.summary.title == "Electricity bill")
        #expect(model.summary.pageCount == 2)
    }

    private var lookups: DocumentLookups {
        DocumentLookups(
            correspondents: [stadtwerke.id: stadtwerke.name], documentTypes: [invoice.id: invoice.name])
    }
}

@MainActor struct UploadViewModelTests {
    @Test func viewerCannotStartAnUpload() async {
        let viewer = Vault(id: UUID(), name: "Household", role: .viewer)
        let service = MockFolioService()
        let destination = VaultDestinationViewModel(
            service: service, catalog: VaultCatalog(vaults: [viewer], defaultVaultID: viewer.id)
        )
        let model = UploadViewModel(service: service, destination: destination) {}

        await model.upload([file("letter.pdf")])

        #expect(model.state == .failed(message: "Choose a vault you can upload to.", uploaded: 0))
        #expect(await service.uploads.isEmpty)
    }

    @Test func changingDestinationDuringAnUploadDoesNotMoveTheRemainingFiles() async {
        let personal = Vault(id: UUID(), name: "Personal", role: .owner)
        let shared = Vault(id: UUID(), name: "Household", role: .contributor)
        let personalService = MockFolioService()
        let sharedService = MockFolioService()
        let catalog = VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id)
        let source = MockFolioService(
            vaults: catalog, vaultServices: [personal.id: personalService, shared.id: sharedService]
        )
        let destination = VaultDestinationViewModel(service: source, catalog: catalog)
        let model = UploadViewModel(service: source, destination: destination) {}
        await personalService.suspendUploads()
        let upload = Task { await model.upload([file("a.pdf"), file("b.pdf")]) }
        await personalService.waitForUploadRequest()

        destination.selectVault(shared.id)
        await personalService.releaseUploads()
        await upload.value

        #expect(await personalService.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(await sharedService.uploads.isEmpty)
        #expect(model.state == .finished(count: 2))
    }

    private func file(_ name: String) -> UploadFile {
        UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
    }

    @Test func uploadsEveryFileAndReportsCount() async {
        let service = MockFolioService()
        var notified = 0
        let model = UploadViewModel(service: service) { notified += 1 }
        await model.upload([file("a.pdf"), file("b.pdf")])
        #expect(model.state == .finished(count: 2))
        #expect(await service.uploads.count == 2)
        #expect(notified == 1)
    }

    @Test func stopsAtFirstFailureAndSaysHowFarItGot() async {
        let service = MockFolioService()
        await service.setFailUploadsAfter(1)
        let model = UploadViewModel(service: service) {}
        await model.upload([file("a.pdf"), file("b.pdf"), file("c.pdf")])
        #expect(model.state == .failed(message: "Couldn't reach the server.", uploaded: 1))
    }

    @Test func retryUploadsOnlyUnfinishedFiles() async {
        let service = MockFolioService()
        await service.setFailUploadsAfter(1)
        var notifications = 0
        let model = UploadViewModel(service: service) { notifications += 1 }
        let files = [file("a.pdf"), file("b.pdf"), file("c.pdf")]
        await model.upload(files)
        await service.setFailUploadsAfter(nil)

        await model.upload(files)

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf", "c.pdf"])
        #expect(model.state == .finished(count: 3))
        #expect(notifications == 2)
    }

    @Test func retryKeepsTheOriginalVaultAfterDestinationChanges() async {
        let personal = Vault(id: UUID(), name: "Personal", role: .owner)
        let shared = Vault(id: UUID(), name: "Household", role: .contributor)
        let personalService = MockFolioService()
        let sharedService = MockFolioService()
        let catalog = VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id)
        let source = MockFolioService(
            vaults: catalog, vaultServices: [personal.id: personalService, shared.id: sharedService]
        )
        let destination = VaultDestinationViewModel(service: source, catalog: catalog)
        let model = UploadViewModel(service: source, destination: destination) {}
        let files = [file("a.pdf"), file("b.pdf")]
        await personalService.setFailUploadsAfter(1)
        await model.upload(files)
        destination.selectVault(shared.id)
        await personalService.setFailUploadsAfter(nil)

        await model.upload(files)

        #expect(await personalService.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(await sharedService.uploads.isEmpty)
        #expect(model.state == .finished(count: 2))
    }

    @Test func differentBatchDoesNotReusePartialProgress() async {
        let service = MockFolioService()
        await service.setFailUploadsAfter(1)
        let model = UploadViewModel(service: service) {}
        await model.upload([file("a.pdf"), file("b.pdf")])
        await service.setFailUploadsAfter(nil)

        await model.upload([file("c.pdf")])

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "c.pdf"])
        #expect(model.state == .finished(count: 1))
    }

    @Test func aSecondBatchWhileOneIsRunningIsIgnored() async {
        let service = MockFolioService()
        await service.setUploadDelay(.milliseconds(50))
        let model = UploadViewModel(service: service) {}
        let first = Task { await model.upload([file("a.pdf"), file("b.pdf")]) }
        while model.state == .idle {
            await Task.yield()
        }
        await model.upload([file("c.pdf")])
        await first.value
        #expect(await service.uploads.map(\.filename).contains("c.pdf") == false)
    }

    @Test func emptySelectionDoesNothing() async {
        let model = UploadViewModel(service: MockFolioService()) {}
        await model.upload([])
        #expect(model.state == .idle)
    }
}

@MainActor struct OnboardingViewModelTests {
    @Test(arguments: [
        ("folio.local:3000", "https://folio.local:3000"),
        ("http://192.168.1.5:3000/", "http://192.168.1.5:3000"),
        ("  https://docs.example.com/folio/ ", "https://docs.example.com/folio"),
    ])
    func normalizesAddresses(input: String, expected: String) throws {
        #expect(try ServerAddress.parse(input).absoluteString == expected)
    }

    @Test(arguments: ["", "   ", "ftp://example.com", "http://"])
    func rejectsUnusableAddresses(input: String) {
        #expect(throws: FolioError.invalidServerURL) { try ServerAddress.parse(input) }
    }

    @Test func connectChecksTheServerThenPersistsTheURL() async {
        let store = MemoryServerConfigStore()
        let service = MockFolioService()
        var connected: URL?
        let model = OnboardingViewModel(store: store, makeService: { _ in service }) {
            connected = $0
        }
        model.address = "folio.local:3000"
        await model.connect()
        #expect(await service.pings == 1)
        #expect(store.url?.absoluteString == "https://folio.local:3000")
        #expect(connected == store.url)
        #expect(model.state == .idle)
    }

    @Test func aHandoffPrefillOpensItsTargetAfterSigningInToThatServer() async throws {
        let server = URL(string: "https://folio.example.com/folio")!
        let route = HandoffRoute(vaultID: UUID(), target: .document(UUID()))
        let window = UUID()
        let inbox = ContinuationInbox()
        inbox.receive(.payload(HandoffPayload(server: server, vaultID: route.vaultID, target: route.target)), scene: window)
        guard case let .signIn(prefill, _) = inbox.decision(isLocked: false, session: .signedOut) else {
            Issue.record("expected a sign-in decision")
            return
        }
        var connected: URL?
        let model = OnboardingViewModel(store: MemoryServerConfigStore(), makeService: { _ in MockFolioService() }) {
            inbox.signedIn(to: $0)
            connected = $0
        }

        model.prefill(prefill)
        #expect(model.address == "https://folio.example.com/folio")
        await model.connect()

        let signedIn = try #require(connected)
        #expect(inbox.take(scene: window, isLocked: false, session: .signedIn(signedIn)) == .open(route))
    }

    @Test func signingInToAnotherServerDropsTheHandoff() async {
        let server = URL(string: "https://folio.example.com")!
        let inbox = ContinuationInbox()
        inbox.receive(.payload(HandoffPayload(server: server, vaultID: UUID(), target: .library(.all))), scene: UUID())
        let model = OnboardingViewModel(store: MemoryServerConfigStore(), makeService: { _ in MockFolioService() }) {
            inbox.signedIn(to: $0)
        }

        model.prefill(server)
        model.address = "other.example.com"
        await model.connect()

        #expect(inbox.pending == nil)
    }

    @Test func signsInBeforeCheckingTheServer() async {
        let service = MockFolioService()
        var events: [String] = []
        let model = OnboardingViewModel(
            store: MemoryServerConfigStore(), makeService: { _ in service },
            signIn: { events.append("sign-in \($0.absoluteString)") }
        ) { _ in }
        model.address = "folio.local"
        await model.connect()
        #expect(events == ["sign-in https://folio.local"])
        #expect(await service.pings == 1)
    }

    @Test func aFailedSignInIsNotPersistedAndSkipsThePing() async {
        let store = MemoryServerConfigStore()
        let service = MockFolioService()
        let model = OnboardingViewModel(
            store: store, makeService: { _ in service },
            signIn: { _ in throw FolioError.signInFailed("Sign-in was cancelled or denied.") }
        ) { _ in }
        model.address = "folio.local"
        await model.connect()
        #expect(store.url == nil)
        #expect(await service.pings == 0)
        #expect(model.state == .failed("Sign-in was cancelled or denied."))
    }

    @Test func aSignedInServerIsPersistedEvenWhenItIsUnreachable() async {
        let store = MemoryServerConfigStore()
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        var connected: URL?
        let model = OnboardingViewModel(store: store, makeService: { _ in service }) { connected = $0 }
        model.address = "folio.local"
        await model.connect()
        // The saved credentials now belong to this server, so the saved address must too.
        #expect(store.url?.absoluteString == "https://folio.local")
        #expect(connected == nil)
        #expect(model.state == .failed("Couldn't reach the server."))
    }

    @Test func invalidAddressFailsBeforeAnyRequest() async {
        let service = MockFolioService()
        let model = OnboardingViewModel(
            store: MemoryServerConfigStore(), makeService: { _ in service }
        ) { _ in }
        model.address = "ftp://nope"
        await model.connect()
        #expect(await service.pings == 0)
        #expect(model.state == .failed("That doesn't look like a server address."))
    }

    @Test func userDefaultsStoreRoundTrips() throws {
        let suite = "folio-test-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = UserDefaultsServerConfigStore(defaults: defaults)
        #expect(store.load() == nil)
        try store.save(#require(URL(string: "https://folio.local")))
        #expect(store.load()?.absoluteString == "https://folio.local")
        store.save(nil)
        #expect(store.load() == nil)
    }
}
