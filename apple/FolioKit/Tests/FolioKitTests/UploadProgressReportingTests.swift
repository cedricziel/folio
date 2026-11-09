import Foundation
import Testing

@testable import FolioKit

private func file(_ name: String) -> UploadFile {
    UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
}

private let tag = Tag(name: "Insurance")

@Suite(.timeLimit(.minutes(1))) struct UploadProgressReportingTests {
    @Test func runReportsEachFileOnceItIsUploadedAndTagged() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let counts = Counts()

        _ = try await runner.run(
            [file("a.pdf"), file("b.pdf"), file("c.pdf")], tagIDs: [tag.id], scope: "s",
            finished: { counts.add($0) })

        #expect(counts.all == [1, 2, 3])
    }

    @Test func aResumedRunCountsTheFilesTheEarlierRunFinished() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf"), file("b.pdf")]
        await service.setFailUploadsAfter(1)
        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run(files, tagIDs: [tag.id], scope: "s")
        }
        await service.setFailUploadsAfter(nil)
        let counts = Counts()

        _ = try await runner.run(files, tagIDs: [tag.id], scope: "s", finished: { counts.add($0) })

        #expect(counts.all == [1, 2])
        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
    }

    @Test func cancellingStopsBeforeTheNextFileAndRunningAgainResumes() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf"), file("b.pdf")]

        // Its own task, so the cancellation it asks for ends that run only.
        let cancelled = Task {
            try await runner.run(
                files, tagIDs: [tag.id], scope: "s", finished: { _ in withUnsafeCurrentTask { $0?.cancel() } })
        }
        await #expect(throws: CancellationError.self) {
            _ = try await cancelled.value
        }
        #expect(await service.uploads.map(\.filename) == ["a.pdf"])
        #expect(runner.progress(files: files, tagIDs: [tag.id], scope: "s")?.uploaded == 1)

        let documents = try await runner.run(files, tagIDs: [tag.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(documents.map(\.originalFilename) == ["a.pdf", "b.pdf"])
        #expect(await service.tagged.map(\.document) == documents.map(\.id))
    }

    @Test func reporterShowsHowManyDocumentsAreDone() {
        let progress = Progress()
        let reporter = UploadProgressReporter(progress, total: 3)
        #expect(progress.totalUnitCount == 3)
        #expect(progress.completedUnitCount == 0)
        #expect(progress.localizedAdditionalDescription == "0 of 3 documents")

        reporter.finished(2)

        #expect(progress.completedUnitCount == 2)
        #expect(progress.localizedAdditionalDescription == "2 of 3 documents")
    }

    @Test func intentsPassCancellationThroughInsteadOfShowingAnError() async {
        IntentEnvironment.use { MockFolioService() }
        IntentEnvironment.flushRuns {}
        await #expect(throws: CancellationError.self) {
            try await IntentEnvironment.run { _ in throw CancellationError() }
        }
    }
}

private final class Counts: @unchecked Sendable {
    private let mutex = NSLock()
    private var counts: [Int] = []

    var all: [Int] { mutex.withLock { counts } }

    func add(_ count: Int) {
        mutex.withLock { counts.append(count) }
    }
}
