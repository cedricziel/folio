import Foundation
import Testing

@testable import FolioKit

@MainActor struct PagingTests {
    private let vault = UUID()

    private func document(_ title: String, filed: Bool = false) -> Document {
        Document(title: title, correspondentID: filed ? UUID() : nil, ocrStatus: .done, embedStatus: .skipped)
    }

    private func page(_ documents: [Document], next: String? = nil) -> DocumentPage {
        DocumentPage(vaultID: vault, documents: documents, nextCursor: next)
    }

    @Test func appendingDeduplicatesAndRetainsSelection() async {
        let a = document("A"), b = document("B"), c = document("C")
        let service = MockFolioService()
        await service.setPages(["|": page([a, b], next: "next"), "|next": page([b, c])])
        let session = DocumentBrowserSession(service: service, destination: .all)
        await session.list.load()
        session.selection = b.id
        #expect(session.list.canLoadMore)
        await session.list.loadNextPage()
        #expect(session.list.rows.map(\.id) == [a.id, b.id, c.id])
        #expect(session.selection == b.id)
        #expect(!session.list.canLoadMore)
    }

    @Test(arguments: [false, true])
    func inboxScansFiledPagesBeforeFinishing(hasReview: Bool) async {
        let review = document("Review")
        let service = MockFolioService()
        await service.setPages([
            "|": page([document("Filed A", filed: true)], next: "two"),
            "|two": page([document("Filed B", filed: true)], next: "three"),
            "|three": page(hasReview ? [review] : [document("Filed C", filed: true)]),
        ])
        let session = DocumentBrowserSession(service: service, destination: .inbox)
        await session.list.load()
        #expect(session.list.rows.map(\.id) == (hasReview ? [review.id] : []))
        #expect(session.list.phase == .loaded)
        #expect(await service.pageRequests == ["|", "|two", "|three"])
    }

    @Test func oldContinuationCannotOverwriteNewSearch() async {
        let service = MockFolioService()
        let old = document("Old"), fresh = document("New")
        await service.setPages(["|": page([old], next: "slow"), "new|": page([fresh])])
        let model = DocumentListViewModel(service: service, searchDebounce: .zero)
        await model.load()
        await service.suspendPage("slow")
        let continuation = Task { await model.loadNextPage() }
        await service.waitForPage()
        model.searchText = "new"
        await model.search("new")
        await service.releasePage(page([document("Stale")], next: "more"))
        await continuation.value
        #expect(model.rows.map(\.id) == [fresh.id])
        #expect(!model.canLoadMore)
        #expect(!model.isLoadingMore)
    }

    @Test func failedContinuationPreservesRowsAndCanBeRetried() async {
        let a = document("A"), b = document("B")
        let service = MockFolioService()
        await service.setPages(["|": page([a], next: "next"), "|next": page([b])])
        let model = DocumentListViewModel(service: service)
        await model.load()
        await service.setFailure(.unreachable)
        await model.loadNextPage()
        #expect(model.rows.map(\.id) == [a.id])
        #expect(model.pageError != nil)
        #expect(model.canLoadMore)
        await service.setFailure(nil)
        await model.loadNextPage()
        #expect(model.rows.map(\.id) == [a.id, b.id])
        #expect(model.pageError == nil)
    }

    @Test func repeatedCursorFailsInsteadOfLoopingThroughFiledPages() async {
        let filed = document("Filed", filed: true)
        let service = MockFolioService()
        await service.setPages(["|": page([filed], next: "same"), "|same": page([filed], next: "same")])
        let session = DocumentBrowserSession(service: service, destination: .inbox)
        await session.list.load()
        guard case .failed = session.list.phase else {
            Issue.record("A repeated cursor must fail rather than declare an empty Inbox")
            return
        }
        #expect(await service.pageRequests.count == 2)
    }

    @Test func deepLinkResolvesADocumentOutsideTheLoadedPage() async {
        let target = document("Outside page")
        let service = MockFolioService(documents: [target])
        await service.setPages(["|": page([document("First page")], next: "next")])
        let model = DocumentListViewModel(service: service)
        await model.load()
        await model.resolveDocument(target.id)
        #expect(model.document(for: target.id) == target)
        #expect(!model.rows.contains(where: { $0.id == target.id }))
    }
    @Test func canceledContinuationCanBeRetried() async {
        let a = document("A"), b = document("B")
        let service = MockFolioService()
        await service.setPages(["|": page([a], next: "next"), "|next": page([b])])
        let model = DocumentListViewModel(service: service)
        await model.load()
        await service.suspendPage("next")
        let request = Task { await model.loadNextPage() }
        await service.waitForPage()
        request.cancel()
        await service.releasePage(page([b]))
        await request.value
        #expect(model.rows.map(\.id) == [a.id])
        #expect(model.canLoadMore && !model.isLoadingMore)
        #expect(model.pageError == nil)
        await model.loadNextPage()
        #expect(model.rows.map(\.id) == [a.id, b.id])
    }

    @Test func continuationCannotMixVaults() async {
        let a = document("A")
        let service = MockFolioService()
        await service.setPages([
            "|": page([a], next: "next"),
            "|next": DocumentPage(vaultID: UUID(), documents: [document("Other vault")]),
        ])
        let model = DocumentListViewModel(service: service)
        await model.load()
        await model.loadNextPage()
        #expect(model.rows.map(\.id) == [a.id])
        #expect(model.pageError != nil)
    }

    @Test func missingDeepLinkShowsAnError() async {
        let model = DocumentListViewModel(service: MockFolioService())
        let id = UUID()
        await model.resolveDocument(id)
        #expect(model.document(for: id) == nil)
        #expect(model.documentError(for: id) != nil)
    }

    @Test func coldDeepLinkLoadsMetadataNamesBeforeOpening() async {
        let sender = Correspondent(name: "Sender")
        let target = Document(title: "Outside page", correspondentID: sender.id)
        let model = DocumentListViewModel(service: MockFolioService(documents: [target], correspondents: [sender]))
        await model.resolveDocument(target.id)
        #expect(model.document(for: target.id) == target)
        #expect(model.correspondentName(for: target) == "Sender")
    }

}
