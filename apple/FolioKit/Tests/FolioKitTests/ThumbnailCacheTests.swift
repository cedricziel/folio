import Foundation
import Testing

@testable import FolioKit

struct ThumbnailCacheTests {
    @Test func cachesSuccessfulBytesAndReloadsChangedDocuments() async throws {
        let service = MockFolioService()
        let first = Data([1, 2, 3])
        await service.setThumbnail(first)
        let cache = ThumbnailCache(service: service, vaultID: UUID())
        var document = Document(title: "Invoice")
        #expect(try await cache.data(for: document) == first)
        await service.setThumbnail(Data([4]))
        #expect(try await cache.data(for: document) == first)
        document.updatedAt = document.updatedAt.addingTimeInterval(1)
        #expect(try await cache.data(for: document) == Data([4]))
        #expect(await service.thumbnailRequests.count == 2)
    }

    @Test func unavailableAndFailedRequestsCanRetry() async throws {
        let service = MockFolioService()
        let cache = ThumbnailCache(service: service, vaultID: UUID())
        let document = Document(title: "Invoice")
        #expect(try await cache.data(for: document) == nil)
        await service.setFailure(.signInRequired)
        await #expect(throws: FolioError.signInRequired) { try await cache.data(for: document) }
        await service.setFailure(nil)
        await service.setThumbnail(Data([1]))
        #expect(try await cache.data(for: document) == Data([1]))
    }

    @Test func separateWorkspacesNeverShareCachedDocuments() async throws {
        let first = MockFolioService(), second = MockFolioService()
        await first.setThumbnail(Data([1]))
        await second.setThumbnail(Data([2]))
        let vault = UUID(), document = Document(title: "Same identifier")
        let firstCache = ThumbnailCache(service: first, vaultID: vault)
        let secondCache = ThumbnailCache(service: second, vaultID: vault)
        #expect(try await firstCache.data(for: document) == Data([1]))
        #expect(try await secondCache.data(for: document) == Data([2]))
    }

    @Test func cancelledCallerDoesNotCancelAnotherRowsPreview() async throws {
        let service = MockFolioService()
        await service.setThumbnail(Data([1]))
        await service.suspendThumbnails()
        let cache = ThumbnailCache(service: service, vaultID: UUID())
        let document = Document(title: "Invoice")
        let first = Task { try await cache.data(for: document) }
        await service.waitForThumbnailRequest()
        let second = Task { try await cache.data(for: document) }
        second.cancel()
        await service.releaseThumbnails()
        #expect(try await first.value == Data([1]))
        await #expect(throws: CancellationError.self) { try await second.value }
        #expect(await service.cancelledThumbnailRequests == 0)
        #expect(await service.thumbnailRequests.count == 1)
    }

    @Test func cancellingOnlyCallerCancelsTransferAndAllowsRetry() async throws {
        let service = MockFolioService()
        await service.setThumbnail(Data([1]))
        await service.suspendThumbnails()
        let cache = ThumbnailCache(service: service, vaultID: UUID())
        let document = Document(title: "Invoice")
        let request = Task { try await cache.data(for: document) }
        await service.waitForThumbnailRequest()
        request.cancel()
        await #expect(throws: CancellationError.self) { try await request.value }
        #expect(await service.cancelledThumbnailRequests == 1)
        await service.releaseThumbnails()
        #expect(try await cache.data(for: document) == Data([1]))
        #expect(await service.thumbnailRequests.count == 2)
    }

    @Test func evictsLeastRecentlyUsedBytesWithinBudget() async throws {
        let service = MockFolioService()
        await service.setThumbnail(Data([1, 2]))
        let cache = ThumbnailCache(service: service, vaultID: UUID(), byteLimit: 3)
        let first = Document(title: "First"), second = Document(title: "Second")
        _ = try await cache.data(for: first)
        _ = try await cache.data(for: second)
        _ = try await cache.data(for: first)
        #expect(await service.thumbnailRequests.count == 3)
    }
}
