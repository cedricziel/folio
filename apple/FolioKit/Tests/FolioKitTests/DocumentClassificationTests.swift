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
        let original = Document(title: "Letter", ocrStatus: .done, embedStatus: .done, canEditDocumentDate: true)
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
        let original = Document(title: "Letter", ocrStatus: .done, embedStatus: .done, canEditDocumentDate: true)
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

    @Test func filingFromTheInboxRemovesTheRowButKeepsTheDocumentOpen() async {
        // Without a review flag the server treats a classified document as filed.
        let original = Document(title: "Letter", ocrStatus: .done, embedStatus: .done, canEditDocumentDate: true)
        let service = await makeService(original)
        let list = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        list.scope = .inbox
        await list.load()
        #expect(list.rows.map(\.id) == [original.id])

        let model = DocumentDetailViewModel(document: original, lookups: list.lookups, service: service)
        await model.classify(.correspondent(city.id))
        list.apply(model.document)
        #expect(list.rows.isEmpty)
        #expect(list.document(for: original.id)?.correspondentID == city.id)

        await model.classify(.correspondent(nil))
        list.apply(model.document)
        #expect(list.rows.map(\.id) == [original.id])
    }

    @Test func reviewTrackingKeepsAClassifiedDocumentInTheInboxUntilReviewed() async throws {
        let original = Document(
            title: "Letter", ocrStatus: .done, embedStatus: .done, reviewed: false, canEditDocumentDate: true,
            canReviewDocument: true)
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
