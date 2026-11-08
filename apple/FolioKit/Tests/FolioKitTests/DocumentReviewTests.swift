import Foundation
import Testing

@testable import FolioKit

struct DocumentReviewTests {
    @Test func explicitReviewDoesNotFollowClassification() {
        let classified = Document(
            title: "Letter", correspondentID: UUID(), ocrStatus: .done, embedStatus: .done, reviewed: false)
        #expect(classified.status == .needsReview)
        let reviewed = Document(title: "Letter", ocrStatus: .done, embedStatus: .done, reviewed: true)
        #expect(reviewed.status == .filed)
    }

    @Test func reviewedProcessingAndFailuresStillNeedAttention() {
        let processing = Document(title: "Letter", ocrStatus: .done, embedStatus: .pending, reviewed: true)
        #expect(processing.status == .indexing)
        let failed = Document(title: "Letter", ocrStatus: .done, embedStatus: .failed, reviewed: true)
        #expect(failed.status == .failed)
    }

    @Test func oldServersRetainClassificationStatusWithoutReviewControls() {
        let old = Document(title: "Letter", correspondentID: UUID(), ocrStatus: .done, embedStatus: .done)
        #expect(old.reviewed == nil)
        #expect(!old.canReviewDocument)
        #expect(old.status == .filed)
    }
}

@MainActor struct DocumentReviewViewModelTests {
    @Test func failedReviewPreservesDocumentAndCanRetry() async throws {
        let service = MockFolioService()
        let original = Document(title: "Letter", reviewed: false, canReviewDocument: true)
        await service.setDocuments([original])
        let model = DocumentDetailViewModel(document: original, service: service)
        await service.setFailure(.unreachable)
        await #expect(throws: FolioError.self) { try await model.saveReview(true) }
        #expect(model.document.reviewed == false)
        #expect(model.reviewFailure != nil)
        #expect(!model.savingReview)
        await service.setFailure(nil)
        try await model.saveReview(true)
        #expect(model.document.reviewed == true)
        #expect(model.reviewFailure == nil)
        #expect(model.document.createdAt == original.createdAt)
        try await model.saveReview(false)
        #expect(model.document.reviewed == false)
    }
}

@MainActor struct DocumentReviewListTests {
    @Test func inboxReviewFilterKeepsReviewedDocuments() async {
        let reviewed = Document(title: "Reviewed", ocrStatus: .done, embedStatus: .done, reviewed: true)
        let service = MockFolioService(documents: [reviewed])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        model.scope = .inbox
        model.filter = DocumentFilter(reviewed: true)
        await model.load()
        #expect(model.rows.map(\.title) == ["Reviewed"])
        model.filter = DocumentFilter()
        await model.load()
        #expect(model.rows.isEmpty)
    }
}

struct DocumentReviewFilterTests {
    @Test func inclusiveBoundsRetainTheLiteralDayAndOtherFilters() throws {
        let tag = UUID()
        let original = DocumentFilter(tagIDs: [tag])
        let updated = try original.applyingReview(false, from: "1582-10-10", to: "1582-10-10")
        #expect(updated.tagIDs == [tag])
        #expect(updated.documentDateFrom?.rawValue == "1582-10-10")
        #expect(updated.documentDateTo?.rawValue == "1582-10-10")
        #expect(updated.reviewed == false)
    }

    @Test func invalidAndReversedBoundsAreRejected() {
        #expect(throws: FolioError.self) { try DocumentFilter().applyingReview(nil, from: "1900-02-29", to: "") }
        #expect(throws: FolioError.self) {
            try DocumentFilter().applyingReview(nil, from: "2026-11-03", to: "2026-11-02")
        }
    }
}
