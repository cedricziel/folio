import Foundation
import Testing

@testable import FolioKit

@MainActor struct DocumentClassificationTests {
    private let city = Correspondent(name: "City utilities")
    private let invoice = DocumentType(name: "Invoice")
    private let taxes = Tag(id: UUID(), name: "Taxes")

    private func makeService(_ document: Document) async -> MockFolioService {
        let service = MockFolioService(documents: [document], correspondents: [city], types: [invoice])
        await service.setTags([taxes])
        return service
    }

    @Test func savingACorrespondentTypeAndTagsShowsTheServerState() async {
        let original = unreviewed()
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.loadClassificationOptions()
        #expect(model.classificationOptions?.correspondents == [city])
        #expect(model.classificationOptions?.documentTypes == [invoice])
        #expect(model.classificationOptions?.tags == [taxes])

        await model.classify(.correspondent(city.id))
        await model.classify(.documentType(invoice.id))
        await model.classify(.addTag(taxes.id))
        #expect(model.document.correspondentID == city.id)
        #expect(model.document.documentTypeID == invoice.id)
        #expect(model.document.tags.map(\.id) == [taxes.id])
        #expect(model.metadata.first { $0.label == "From" }?.value == "City utilities")
        #expect(model.metadata.first { $0.label == "Type" }?.value == "Invoice")
        #expect(model.classificationFailure == nil)
        #expect(!model.savingClassification)

        await model.classify(.removeTag(taxes.id))
        await model.classify(.correspondent(nil))
        #expect(model.document.tags.isEmpty)
        #expect(model.document.correspondentID == nil)
        #expect(await service.tagged == [TagAssignment(tag: taxes.id, document: original.id)])
        #expect(await service.untagged == [TagAssignment(tag: taxes.id, document: original.id)])
    }

    @Test func failedSaveKeepsTheDocumentAndRetryAppliesTheChange() async {
        let original = unreviewed()
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await service.setFailure(.unreachable)
        await model.classify(.correspondent(city.id))
        #expect(model.document.correspondentID == nil)
        #expect(model.classificationFailure == "Couldn't reach the server.")
        #expect(model.canRetryClassification)
        #expect(!model.savingClassification)

        await service.setFailure(nil)
        await model.retryClassification()
        #expect(model.document.correspondentID == city.id)
        #expect(model.classificationFailure == nil)
        #expect(!model.canRetryClassification)
    }

    @Test func failedOptionsLoadCanBeRetried() async {
        let original = Document(title: "Letter", canEditDocumentDate: true)
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await service.setFailLookups(true)
        await model.loadClassificationOptions()
        #expect(model.classificationOptions == nil)
        #expect(model.optionsFailure == "Couldn't reach the server.")
        await service.setFailLookups(false)
        await model.loadClassificationOptions()
        #expect(model.classificationOptions?.correspondents == [city])
        #expect(model.optionsFailure == nil)
    }

    @Test func classificationFollowsTheServersEditPermission() {
        let service = MockFolioService()
        #expect(!DocumentDetailViewModel(document: Document(title: "Shared"), service: service).canClassify)
        let editable = Document(title: "Mine", canEditDocumentDate: true)
        #expect(DocumentDetailViewModel(document: editable, service: service).canClassify)
    }

    private func unreviewed() -> Document {
        Document(
            title: "Letter", ocrStatus: .done, embedStatus: .done, canEditDocumentDate: true, reviewed: false,
            canReviewDocument: true)
    }

    @Test func aWriteThatSucceedsWithAFailedFetchIsNotAFailedSave() async {
        let original = unreviewed()
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await service.setFailDocumentReads(.unreachable)
        await model.classify(.correspondent(city.id))
        #expect(model.classificationFailure == nil)
        #expect(!model.canRetryClassification)
        #expect(model.metadataFailure == "Couldn't reach the server.")

        await service.setFailDocumentReads(nil)
        await model.refreshMetadata()
        #expect(model.document.correspondentID == city.id)
        #expect(model.metadataFailure == nil)
    }

    @Test func aRefreshStartedBeforeASaveCannotUndoIt() async {
        let original = unreviewed()
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await service.suspendDocumentReads()
        let refresh = Task { await model.refreshMetadata() }
        await service.waitForDocumentRead()
        await service.stopSuspendingDocumentReads()

        await model.classify(.correspondent(city.id))
        #expect(model.document.correspondentID == city.id)
        await service.releaseDocumentReads()
        await refresh.value
        #expect(model.document.correspondentID == city.id)
    }

    @Test func aListRowFromTheSavedDocumentDoesNotFetchItAgain() async {
        let original = unreviewed()
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.refreshMetadata(after: original.updatedAt)
        await model.classify(.correspondent(city.id))
        let reads = await service.documentReads
        await model.refreshMetadata(after: model.document.updatedAt)
        #expect(await service.documentReads == reads)

        var newer = model.document
        newer.updatedAt = newer.updatedAt.addingTimeInterval(60)
        await service.setDocuments([newer])
        await model.refreshMetadata(after: newer.updatedAt)
        #expect(await service.documentReads == reads + 1)
    }

    @Test func reopeningClassificationClearsAStaleFailure() async {
        let original = unreviewed()
        let service = await makeService(original)
        let model = DocumentDetailViewModel(document: original, service: service)
        await service.setFailure(.unreachable)
        await model.classify(.correspondent(city.id))
        #expect(model.canRetryClassification)
        await service.setFailure(nil)
        await model.loadClassificationOptions()
        #expect(!model.canRetryClassification)
        #expect(model.classificationFailure == nil)
    }

    @Test func aFiledDocumentStaysOpenAfterAReloadDropsItFromTheInbox() async throws {
        let original = unreviewed()
        let service = await makeService(original)
        let list = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        list.scope = .inbox
        await list.load()
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.classify(.correspondent(city.id))
        try await model.saveReview(true)
        list.apply(model.document)
        #expect(list.rows.isEmpty)

        await list.load()
        #expect(list.rows.isEmpty)
        #expect(list.document(for: original.id)?.reviewed == true)
        #expect(list.document(for: original.id)?.correspondentID == city.id)
    }

    @Test func aReloadOlderThanAnAppliedSaveKeepsTheSave() async throws {
        let original = unreviewed()
        let service = await makeService(original)
        let list = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        list.scope = .inbox
        await list.load()
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.classify(.correspondent(city.id))
        try await model.saveReview(true)
        list.apply(model.document)

        // The page was read before the save landed.
        await service.setPages(["|": DocumentPage(vaultID: UUID(), documents: [original])])
        await list.load()
        #expect(list.rows.isEmpty)
        #expect(list.document(for: original.id)?.reviewed == true)
    }

    /// The server files a document only when it is reviewed, never because it was classified.
    @Test func aClassifiedDocumentStaysInTheInboxUntilReviewed() async throws {
        let original = unreviewed()
        let service = await makeService(original)
        let list = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        list.scope = .inbox
        await list.load()
        let model = DocumentDetailViewModel(document: original, service: service)
        await model.classify(.correspondent(city.id))
        list.apply(model.document)
        #expect(list.rows.map(\.id) == [original.id])
        #expect(list.rows.first?.subtitle == "City utilities")

        try await model.saveReview(true)
        list.apply(model.document)
        #expect(list.rows.isEmpty)
    }

    @Test func olderDocumentsDoNotReplaceNewerListData() async {
        let original = Document(title: "Letter", ocrStatus: .done, embedStatus: .done)
        let service = MockFolioService(documents: [original])
        let list = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        await list.load()
        var stale = original
        stale.title = "Stale"
        stale.updatedAt = original.updatedAt.addingTimeInterval(-60)
        list.apply(stale)
        #expect(list.rows.map(\.title) == ["Letter"])
    }
}
