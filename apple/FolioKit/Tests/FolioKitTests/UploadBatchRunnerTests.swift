import Foundation
import Testing

@testable import FolioKit

private func file(_ name: String) -> UploadFile {
    UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
}

private let first = Tag(name: "Insurance")
private let second = Tag(name: "Taxes")

struct UploadBatchRunnerTests {
    @Test func uploadSucceedsTaggingFailsThenRetryAttachesOnlyTheRemainingTag() async throws {
        let service = MockFolioService()
        let store = InMemoryUploadProgressStore()
        let runner = UploadBatchRunner(service: service, store: store)
        let files = [file("a.pdf")]
        await service.setFailingTags([second.id])

        await #expect(throws: FolioError.server(status: 500, message: nil)) {
            _ = try await runner.run(files, tagIDs: [first.id, second.id], scope: "s")
        }
        let stopped = try #require(runner.progress(files: files, tagIDs: [first.id, second.id], scope: "s"))
        #expect(stopped.uploaded == 1)
        #expect(stopped.remainingTags == 1)

        await service.setFailingTags([])
        let documents = try await runner.run(files, tagIDs: [first.id, second.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf"])
        let tagged = await service.tagged
        #expect(tagged.map(\.tag) == [first.id, second.id])
        #expect(Set(tagged.map(\.document)) == [try #require(documents.first?.id)])
        #expect(runner.progress(files: files, tagIDs: [first.id, second.id], scope: "s") == nil)
    }

    @Test func secondFileFailsThenRetryUploadsOnlyThatFile() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf"), file("b.pdf")]
        await service.setFailUploadsAfter(1)

        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run(files, tagIDs: [first.id], scope: "s")
        }
        #expect(runner.progress(files: files, tagIDs: [first.id], scope: "s")?.uploaded == 1)

        await service.setFailUploadsAfter(nil)
        let documents = try await runner.run(files, tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(documents.map(\.originalFilename) == ["a.pdf", "b.pdf"])
        #expect(await service.tagged.map(\.document) == documents.map(\.id))
    }

    @Test func lostUploadAnswerIsReplayedByItsKeyInsteadOfUploadedAgain() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf")]
        await service.setLoseUploadResponses(true)

        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run(files, tagIDs: [first.id], scope: "s")
        }
        let stored = try #require(await service.documents(matching: "").first)

        await service.setLoseUploadResponses(false)
        let documents = try await runner.run(files, tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.count == 1)
        let keys = await service.uploadKeys
        #expect(keys.count == 2)
        #expect(Set(keys).count == 1, "the retry reuses the first attempt's Idempotency-Key")
        #expect(documents.map(\.id) == [stored.id])
        #expect(await service.tagged.map(\.document) == [stored.id])
    }

    @Test func aSameNamedDocumentFromEarlierIsNeverAdopted() async throws {
        let earlier = Document(title: "Scan", originalFilename: "scan.pdf", createdAt: .now)
        let service = MockFolioService(documents: [earlier])
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setFailure(.unreachable)
        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run([file("scan.pdf")], tagIDs: [first.id], scope: "s")
        }

        await service.setFailure(nil)
        let documents = try await runner.run([file("scan.pdf")], tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["scan.pdf"])
        #expect(documents.first?.id != earlier.id)
        #expect(await service.tagged.map(\.document) != [earlier.id])
    }

    @Test func aDocumentDeletedBeforeTaggingIsUploadedAgain() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setFailingTags([first.id])
        _ = try? await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")
        let deleted = try #require(runner.progress(files: [file("a.pdf")], tagIDs: [first.id], scope: "s"))
        await service.markDeleted(try #require(deleted.steps.first?.documentID))

        await service.setFailingTags([])
        let documents = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.count == 2)
        #expect(Set(await service.uploadKeys).count == 2, "the new upload gets a new key")
        let replacement = try #require(documents.first?.id)
        #expect(replacement != deleted.steps.first?.documentID)
        #expect(await service.tagged.map(\.document) == [replacement])
    }

    @Test func aKeyWhoseDocumentWasDeletedUploadsTheFileAgain() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setLoseUploadResponses(true)
        _ = try? await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")
        let lost = try #require(await service.documents(matching: "").first)
        await service.markDeleted(lost.id)

        await service.setLoseUploadResponses(false)
        let documents = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.count == 2)
        let replacement = try #require(documents.first?.id)
        #expect(replacement != lost.id)
        #expect(await service.tagged.map(\.document) == [replacement])
    }

    @Test func aDeletedTagFailsWithoutUploadingAgain() async throws {
        let service = MockFolioService()
        await service.setKeepsUploads(true)
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setMissingTags([second.id])

        await #expect(throws: UploadBatchError.tagNoLongerExists) {
            _ = try await runner.run([file("a.pdf")], tagIDs: [first.id, second.id], scope: "s")
        }
        await #expect(throws: UploadBatchError.tagNoLongerExists) {
            _ = try await runner.run([file("a.pdf")], tagIDs: [first.id, second.id], scope: "s")
        }

        #expect(await service.uploads.count == 1, "a missing tag never causes another upload")
        let progress = try #require(runner.progress(files: [file("a.pdf")], tagIDs: [first.id, second.id], scope: "s"))
        #expect(progress.uploaded == 1)
        #expect(progress.remainingTags == 1)
    }

    @Test func aBatchAlreadyRunningElsewhereIsNotStartedTwice() async throws {
        let service = MockFolioService()
        let store = InMemoryUploadProgressStore()
        let runner = UploadBatchRunner(service: service, store: store)
        let key = UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "s")
        #expect(store.lock(key))

        await #expect(throws: UploadBatchError.alreadyRunning) {
            _ = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")
        }
        #expect(await service.uploads.isEmpty)

        store.unlock(key)
        _ = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")
        #expect(await service.uploads.count == 1)
    }

    @Test func aDifferentBatchDoesNotReuseProgress() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setFailUploadsAfter(1)
        _ = try? await runner.run([file("a.pdf"), file("b.pdf")], tagIDs: [first.id], scope: "s")
        await service.setFailUploadsAfter(nil)

        _ = try await runner.run([file("a.pdf"), file("b.pdf")], tagIDs: [second.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "a.pdf", "b.pdf"])
    }

    @Test func keysSeparateServersTagsAndContents() {
        let base = UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "s")
        #expect(base == UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "s"))
        #expect(base != UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "other"))
        #expect(base != UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [second.id], scope: "s"))
        var changed = file("a.pdf")
        changed.data = Data("different".utf8)
        #expect(base != UploadBatchRunner.key(files: [changed], tagIDs: [first.id], scope: "s"))
    }

    @Test func fileStoreForgetsStaleProgress() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let progress = UploadBatchProgress(
            steps: [UploadStep(documentID: UUID(), remainingTags: [first.id])], updatedAt: .now)
        FileUploadProgressStore(directory: directory).save(progress, for: "batch")

        #expect(FileUploadProgressStore(directory: directory).load("batch") == progress)
        #expect(FileUploadProgressStore(directory: directory, maxAge: -1).load("batch") == nil)
        #expect(FileUploadProgressStore(directory: directory).load("batch") == nil)
    }

    @Test func savingSweepsAbandonedProgress() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = FileUploadProgressStore(directory: directory)
        let progress = UploadBatchProgress(steps: [UploadStep(remainingTags: [])], updatedAt: .now)
        store.save(progress, for: "abandoned")
        let abandoned = directory.appending(path: "upload-progress/abandoned.json")
        try FileManager.default.setAttributes(
            [.modificationDate: Date.now.addingTimeInterval(-2 * 24 * 60 * 60)], ofItemAtPath: abandoned.path)

        store.save(progress, for: "current")

        #expect(!FileManager.default.fileExists(atPath: abandoned.path))
        #expect(store.load("current") == progress)
    }

    @Test(.timeLimit(.minutes(1))) func aLongRunRenewsItsLock() async throws {
        let service = MockFolioService()
        await service.suspendUploads()
        let store = RenewalCountingStore()
        let runner = UploadBatchRunner(service: service, store: store, lockRenewal: .milliseconds(10))

        // The run renews once before it sends the upload. The upload then stays open until the timer has renewed
        // too, however slowly it runs.
        let run = Task { try await runner.run([file("a.pdf")], tagIDs: [], scope: "s") }
        await service.waitForUploadRequest()
        await store.waitForRenewals(2)
        await service.releaseUploads()
        _ = try await run.value
        let renewals = store.renewals

        // A renewal already past its sleep when the run ends may still land; none may follow it.
        try await Task.sleep(for: .milliseconds(100))
        #expect(store.renewals <= renewals + 1, "renewal stops with the run")
    }

    @Test func aRunThatLosesItsLockStopsBeforeItsNextRequest() async throws {
        let service = MockFolioService()
        // Held for the upload and the tag of the first file; taken over before the second file.
        let store = RenewalCountingStore(heldRenewals: 2)
        let runner = UploadBatchRunner(service: service, store: store)
        let files = [file("a.pdf"), file("b.pdf")]

        await #expect(throws: UploadBatchError.lockLost) {
            _ = try await runner.run(files, tagIDs: [first.id], scope: "s")
        }

        #expect(await service.uploads.map(\.filename) == ["a.pdf"])
        #expect(await service.tagged.count == 1)
    }

    @Test func aRunSuspendedPastATakeoverSendsNothingMore() async throws {
        let service = MockFolioService()
        let store = RenewalCountingStore(heldRenewals: 0)
        let runner = UploadBatchRunner(service: service, store: store)

        await #expect(throws: UploadBatchError.lockLost) {
            _ = try await runner.run([file("a.pdf")], tagIDs: [], scope: "s")
        }

        #expect(await service.uploads.isEmpty)
        #expect(runner.progress(files: [file("a.pdf")], tagIDs: [], scope: "s") == nil, "nothing recorded")
    }

    @Test func anUploadStillBeingStoredIsTriedAgainAfterTheServersWait() async throws {
        let service = MockFolioService()
        await service.setUploadConflicts([.inProgress(retryAfter: .seconds(7))])
        let waits = Waits()
        let runner = UploadBatchRunner(
            service: service, store: InMemoryUploadProgressStore(), sleep: { waits.add($0) })

        let documents = try await runner.run([file("a.pdf")], tagIDs: [], scope: "s")

        #expect(documents.count == 1)
        #expect(waits.all == [.seconds(7)])
        let keys = await service.uploadKeys
        #expect(keys.count == 2 && keys[0] == keys[1], "the retry keeps the key")
    }

    @Test func theWaitIsCappedAndTriedOnlyOnce() async throws {
        let service = MockFolioService()
        await service.setUploadConflicts([.inProgress(retryAfter: .seconds(600)), .inProgress(retryAfter: nil)])
        let waits = Waits()
        let store = InMemoryUploadProgressStore()
        let runner = UploadBatchRunner(service: service, store: store, sleep: { waits.add($0) })

        await #expect(throws: KeyedUploadConflict.inProgress(retryAfter: nil)) {
            _ = try await runner.run([file("a.pdf")], tagIDs: [], scope: "s")
        }

        #expect(waits.all == [UploadBatchRunner.maxConflictWait])
        let kept = try #require(runner.progress(files: [file("a.pdf")], tagIDs: [], scope: "s"))
        #expect(kept.steps.first?.idempotencyKey == (await service.uploadKeys).first, "the next run keeps the key")
    }

    @Test func aKeyUsedForAnotherFileIsReplaced() async throws {
        let service = MockFolioService()
        await service.setUploadConflicts([.keyReused])
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())

        _ = try await runner.run([file("a.pdf")], tagIDs: [], scope: "s")

        let keys = await service.uploadKeys
        #expect(keys.count == 2 && keys[0] != keys[1])
        #expect(await service.uploads.count == 1)
    }

    @Test func fileStoreRenewalKeepsARunningBatchFromBeingTakenOver() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let running = FileUploadProgressStore(directory: directory)
        let later = Date.now.addingTimeInterval(FileUploadProgressStore.lockTimeout + 60)
        #expect(running.lock("batch"))

        let lock = directory.appending(path: "upload-progress/batch.lock.1")
        try FileManager.default.setAttributes(
            [.modificationDate: Date.now.addingTimeInterval(-FileUploadProgressStore.lockTimeout - 60)],
            ofItemAtPath: lock.path)
        #expect(running.renew("batch"))
        #expect(!FileUploadProgressStore(directory: directory).lock("batch", now: .now))

        let other = FileUploadProgressStore(directory: directory)
        #expect(other.lock("batch", now: later), "a lock nobody renews is taken over")
        #expect(!running.renew("batch"), "the run that lost its lock learns it at its next renewal")
        running.unlock("batch")
        #expect(!FileUploadProgressStore(directory: directory).lock("batch", now: .now))
        other.unlock("batch")
        #expect(FileUploadProgressStore(directory: directory).lock("batch"))
    }

    @Test func fileStoreLocksABatchAcrossInstances() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }

        let holder = FileUploadProgressStore(directory: directory)
        #expect(holder.lock("batch"))
        #expect(!FileUploadProgressStore(directory: directory).lock("batch"))
        FileUploadProgressStore(directory: directory).unlock("batch")
        #expect(!FileUploadProgressStore(directory: directory).lock("batch"), "only the holder releases the lock")
        holder.unlock("batch")
        #expect(FileUploadProgressStore(directory: directory).lock("batch"))
    }
}

struct ShortcutUploadRetryTests {
    private func catalog(_ service: MockFolioService, store: any UploadProgressStoring) -> DocumentCatalog {
        DocumentCatalog(service: service, progress: store, uploadScope: "https://folio.example.com")
    }

    /// Each Shortcut run builds a new catalog; the progress file carries over between them.
    @Test func rerunningAShortcutFinishesTaggingWithoutUploadingAgain() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let service = MockFolioService()
        await service.setFailingTags([second.id])

        await #expect(throws: FolioError.self) {
            _ = try await catalog(service, store: FileUploadProgressStore(directory: directory))
                .upload([file("a.pdf")], title: "Policy", tagIDs: [first.id, second.id])
        }
        await service.setFailingTags([])
        let summaries = try await catalog(service, store: FileUploadProgressStore(directory: directory))
            .upload([file("a.pdf")], title: "Policy", tagIDs: [first.id, second.id])

        #expect(await service.uploads.map(\.title) == ["Policy"])
        #expect(await service.tagged.map(\.tag) == [first.id, second.id])
        #expect(summaries.count == 1)
    }

    @Test func rerunningAShortcutUploadsOnlyTheFilesThatFailed() async throws {
        let service = MockFolioService()
        let store = InMemoryUploadProgressStore()
        await service.setFailUploadsAfter(1)
        await #expect(throws: FolioError.unreachable) {
            _ = try await catalog(service, store: store).upload([file("a.pdf"), file("b.pdf")], tagIDs: [first.id])
        }

        await service.setFailUploadsAfter(nil)
        let summaries = try await catalog(service, store: store)
            .upload([file("a.pdf"), file("b.pdf")], tagIDs: [first.id])

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(await service.tagged.count == 2)
        #expect(summaries.count == 2)
    }
}

/// Through the real HTTP client: the upload is accepted, the tag request fails, and the retry only tags.
@Suite(.timeLimit(.minutes(1))) struct UploadBatchRunnerHTTPTests {
    private let documentID = "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f"

    private let documentJSON = """
        {"id": "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f", "title": "Jahresabrechnung 2025",
         "vault_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "uploaded_by": "fixture-user",
         "created_by": "archive-creator", "updated_by": "archive-editor",
         "content": "Rechnungsnummer 2025-00318", "original_filename": "bill.pdf",
         "file_path": "a/bill.pdf", "mime_type": "application/pdf", "correspondent_id": null,
         "document_type_id": "11111111-2222-4333-8444-555555555555", "page_count": 3, "file_size": 48213,
         "ocr_status": "done", "embed_status": "skipped",
         "correspondent": null, "document_type": "Invoice",
         "tags": [], "created_at": "2025-03-14T10:15:30.123456789Z", "updated_at": "2025-03-14T10:15:30Z"}
        """

    @Test func tagFailureRetrySendsOnlyTheTagRequest() async throws {
        let host = "batch-\(UUID().uuidString.lowercased()).example.com"
        ShareUploadProtocol.serve(host, [.json(201, documentJSON), .status(500), .status(204)])
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ShareUploadProtocol.self]
        let service = LiveFolioService(
            baseURL: URL(string: "https://\(host)")!, tokenProvider: RotatingTokens(),
            session: URLSession(configuration: configuration)
        )
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf")]

        await #expect(throws: FolioError.self) { _ = try await runner.run(files, tagIDs: [first.id], scope: host) }
        let documents = try await runner.run(files, tagIDs: [first.id], scope: host)

        let tags = "/api/documents/\(documentID)/tags"
        let paths = ShareUploadProtocol.requests(to: host).map { $0.url?.path() ?? "" }
        #expect(Array(paths.prefix(3)) == ["/api/documents/upload", tags, tags])
        let upload = try #require(ShareUploadProtocol.requests(to: host).first)
        #expect(upload.value(forHTTPHeaderField: "Idempotency-Key").flatMap(UUID.init(uuidString:)) != nil)
        #expect(!paths.dropFirst(3).contains("/api/documents/upload"))
        #expect(documents.map(\.id.uuidString).map { $0.lowercased() } == [documentID])
    }
}

/// Counts lock renewals of an in-memory store; renewals after the first `heldRenewals` report the lock lost.
private final class RenewalCountingStore: UploadProgressStoring, @unchecked Sendable {
    private let base = InMemoryUploadProgressStore()
    private let heldRenewals: Int
    private let mutex = NSLock()
    private var count = 0
    private var waiters: [(renewals: Int, continuation: CheckedContinuation<Void, Never>)] = []

    init(heldRenewals: Int = .max) {
        self.heldRenewals = heldRenewals
    }

    var renewals: Int { mutex.withLock { count } }

    /// Returns once the lock has been renewed `renewals` times.
    func waitForRenewals(_ renewals: Int) async {
        await withCheckedContinuation { waiter in
            let renewed = mutex.withLock {
                if count >= renewals { return true }
                waiters.append((renewals, waiter))
                return false
            }
            if renewed { waiter.resume() }
        }
    }

    func load(_ key: String) -> UploadBatchProgress? { base.load(key) }
    func save(_ progress: UploadBatchProgress, for key: String) { base.save(progress, for: key) }
    func remove(_ key: String) { base.remove(key) }
    func lock(_ key: String) -> Bool { base.lock(key) }
    func renew(_: String) -> Bool {
        let (held, waiting) = mutex.withLock {
            count += 1
            let ready = waiters.filter { $0.renewals <= count }
            waiters.removeAll { $0.renewals <= count }
            return (count <= heldRenewals, ready)
        }
        waiting.forEach { $0.continuation.resume() }
        return held
    }
    func unlock(_ key: String) { base.unlock(key) }
}

/// Records the waits a runner asked for instead of sleeping.
private final class Waits: @unchecked Sendable {
    private let mutex = NSLock()
    private var waits: [Duration] = []

    var all: [Duration] { mutex.withLock { waits } }

    func add(_ wait: Duration) {
        mutex.withLock { waits.append(wait) }
    }
}
