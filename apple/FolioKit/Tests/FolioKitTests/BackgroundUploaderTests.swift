import Foundation
import Testing

@testable import FolioKit

/// Answers share uploads by host, so tests running in parallel never consume each other's replies.
final class ShareUploadProtocol: URLProtocol, @unchecked Sendable {
    enum Reply: Sendable {
        case status(Int)
        case json(Int, String)
        case transport(URLError.Code)
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var replies: [String: [Reply]] = [:]
    nonisolated(unsafe) private static var received: [String: [URLRequest]] = [:]

    static func serve(_ host: String, _ queued: [Reply]) {
        lock.withLock {
            replies[host] = queued
            received[host] = []
        }
    }

    static func requests(to host: String) -> [URLRequest] {
        lock.withLock { received[host] ?? [] }
    }

    private static func next(for request: URLRequest) -> Reply {
        let host = request.url?.host() ?? ""
        return lock.withLock {
            received[host, default: []].append(request)
            guard replies[host]?.isEmpty == false else { return .status(500) }
            return replies[host]!.removeFirst()
        }
    }

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let reply = Self.next(for: request)
        switch reply {
        case let .transport(code):
            client?.urlProtocol(self, didFailWithError: URLError(code))
        case let .status(status):
            answer(status, body: "{}")
        case let .json(status, body):
            answer(status, body: body)
        }
    }

    private func answer(_ status: Int, body: String) {
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

/// Hands out `stale` until asked to refresh, as if the server had revoked it.
final class RotatingTokens: BearerTokenProvider, @unchecked Sendable {
    private let lock = NSLock()
    private var current = "stale"
    private var rejectedTokens: [String?] = []

    var rejected: [String?] { lock.withLock { rejectedTokens } }

    func token() async throws -> String? {
        currentToken()
    }

    func refresh(rejecting token: String?) async throws -> String? {
        rotate(rejecting: token)
    }

    private func currentToken() -> String {
        lock.withLock { current }
    }

    private func rotate(rejecting token: String?) -> String {
        lock.withLock {
            rejectedTokens.append(token)
            current = "fresh"
            return current
        }
    }
}

/// One uploader over a foreground session that `ShareUploadProtocol` answers, with its own store directory.
struct ShareUploadHarness: Sendable {
    let host = "share-\(UUID().uuidString.lowercased()).example.com"
    let directory: URL
    let tokens = RotatingTokens()
    let uploader: BackgroundUploader
    let outcomes: AsyncStream<ShareUploadRecord>

    init(_ replies: [ShareUploadProtocol.Reply]) {
        directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-share-\(UUID().uuidString)", directoryHint: .isDirectory)
        uploader = Self.uploader(host: host, tokens: tokens, directory: directory)
        let (stream, continuation) = AsyncStream.makeStream(of: ShareUploadRecord.self)
        outcomes = stream
        uploader.onChange = { continuation.yield($0) }
        ShareUploadProtocol.serve(host, replies)
    }

    static func uploader(
        host: String, tokens: RotatingTokens, directory: URL, identifier: String = "test"
    ) -> BackgroundUploader {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ShareUploadProtocol.self]
        return BackgroundUploader(
            serverURL: URL(string: "https://\(host)")!, tokens: tokens, identifier: identifier,
            directory: directory, configuration: configuration
        )
    }

    /// A pending share as an earlier process left it, with its body on disk.
    func savePending(sessionIdentifier: String?, minutesAgo: Double) throws -> ShareUploadRecord {
        let record = ShareUploadRecord(
            id: UUID(), filename: "scan.pdf", contentType: "multipart/form-data; boundary=x", status: .pending,
            attempts: 1, updatedAt: .now.addingTimeInterval(-minutesAgo * 60),
            serverURL: URL(string: "https://\(host)"), sessionIdentifier: sessionIdentifier
        )
        try uploader.store.save(record)
        try Data("--x--".utf8).write(to: uploader.store.bodyFile(for: record.id))
        return record
    }

    func nextOutcome() async -> ShareUploadRecord? {
        var iterator = outcomes.makeAsyncIterator()
        return await iterator.next()
    }

    func bodyExists(_ record: ShareUploadRecord) -> Bool {
        FileManager.default.fileExists(atPath: uploader.store.bodyFile(for: record.id).path)
    }

    func tearDown() {
        uploader.invalidate()
        try? FileManager.default.removeItem(at: directory)
    }
}

private func pdf(_ name: String = "scan.pdf") -> UploadFile {
    UploadFile(data: Data("%PDF-1.4".utf8), filename: name, mimeType: "application/pdf", title: "Scan")
}

struct ShareUploadFailureTests {
    private func response(_ status: Int) -> HTTPURLResponse {
        HTTPURLResponse(
            url: URL(string: "https://folio.example.com")!, statusCode: status, httpVersion: nil, headerFields: nil
        )!
    }

    @Test func acceptsOnlySuccessfulStatuses() {
        #expect(ShareUploadFailure.classify(response: response(201), error: nil) == nil)
        #expect(ShareUploadFailure.classify(response: response(401), error: nil) == .unauthorized)
        #expect(ShareUploadFailure.classify(response: response(500), error: nil) == .rejected(status: 500))
        #expect(ShareUploadFailure.classify(response: response(415), error: nil) == .rejected(status: 415))
        #expect(ShareUploadFailure.classify(response: nil, error: nil) != nil)
    }

    @Test func transportErrorsWithoutAnAcceptedResponseFail() {
        let failure = ShareUploadFailure.classify(response: response(500), error: URLError(.networkConnectionLost))
        guard case .transport = failure else {
            Issue.record("Expected a transport failure, got \(String(describing: failure))")
            return
        }
    }

    /// The server stored the file; a transport error after its answer must not offer a retry.
    @Test func anAcceptedResponseWinsOverALateTransportError() {
        #expect(ShareUploadFailure.classify(response: response(201), error: URLError(.networkConnectionLost)) == nil)
    }

    @Test func onlyRepeatableFailuresAreRetryable() {
        #expect(ShareUploadFailure.unauthorized.isRetryable)
        #expect(ShareUploadFailure.transport("offline").isRetryable)
        #expect(ShareUploadFailure.rejected(status: 503).isRetryable)
        #expect(ShareUploadFailure.rejected(status: 429).isRetryable)
        #expect(
            ShareUploadFailure.rejected(status: 409).isRetryable,
            "the server answers 409 while an earlier attempt with the same key is still storing the file")
        #expect(!ShareUploadFailure.rejected(status: 415).isRetryable)
        #expect(!ShareUploadFailure.rejected(status: 413).isRetryable)
        #expect(!ShareUploadFailure.rejected(status: 410).isRetryable)
    }

    @Test func aReusedKeyIsAPermanentRefusal() throws {
        let reused = Data(#"{"error":"used","code":"idempotency_key_mismatch"}"#.utf8)
        let busy = Data(#"{"error":"busy","code":"upload_in_progress"}"#.utf8)

        #expect(ShareUploadFailure.classify(response: response(409), error: nil, body: reused) == .keyReused)
        #expect(ShareUploadFailure.classify(response: response(409), error: nil, body: busy) == .rejected(status: 409))
        #expect(
            ShareUploadFailure.classify(response: response(409), error: nil) == .rejected(status: 409),
            "an answer whose body never arrived stays retryable")
        #expect(!ShareUploadFailure.keyReused.isRetryable)
        #expect(ShareUploadFailure.keyReused.message.contains("Share the file again"))
    }

    @Test func keyedConflictsReadTheirCodeAndWait() throws {
        func conflict(_ status: Int, _ headers: [String: String], _ body: String) throws -> KeyedUploadConflict? {
            let url = try #require(URL(string: "https://folio.example.com"))
            let response = try #require(
                HTTPURLResponse(url: url, statusCode: status, httpVersion: nil, headerFields: headers))
            return KeyedUploadConflict(response: response, body: Data(body.utf8))
        }

        #expect(try conflict(409, [:], #"{"code":"idempotency_key_mismatch"}"#) == .keyReused)
        #expect(
            try conflict(409, ["Retry-After": "5"], #"{"code":"upload_in_progress"}"#)
                == .inProgress(retryAfter: .seconds(5)))
        #expect(try conflict(409, [:], "not json") == .inProgress(retryAfter: nil), "servers before the codes")
        #expect(try conflict(409, [:], #"{"code":"something_new"}"#) == nil, "an unknown code keeps its message")
        #expect(try conflict(410, [:], #"{"code":"upload_deleted"}"#) == nil)
    }

    @Test func keyedConflictsExplainWhatToDo() {
        #expect(ShareUploadFailure.rejected(status: 409).message.contains("still filing"))
        #expect(ShareUploadFailure.rejected(status: 410).message.contains("deleted"))
        #expect(ShareUploadFailure.rejected(status: 410).message.contains("Share the file again"))
    }
}

struct ShareUploadStoreTests {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "folio-share-\(UUID().uuidString)", directoryHint: .isDirectory)

    private func saved(
        _ store: ShareUploadStore, _ status: ShareUploadRecord.Status, attempts: Int = 1, minutesAgo: Double = 0,
        bodyBytes: Int = 4
    ) throws -> ShareUploadRecord {
        let record = ShareUploadRecord(
            id: UUID(), filename: "a.pdf", contentType: "multipart/form-data", status: status, attempts: attempts,
            updatedAt: .now.addingTimeInterval(-minutesAgo * 60)
        )
        try store.save(record)
        try Data(count: bodyBytes).write(to: store.bodyFile(for: record.id))
        return record
    }

    @Test func anAcceptedUploadIsNeverDowngraded() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let record = try saved(store, .uploaded, attempts: 2)

        #expect(store.complete(record.id, attempt: 2, failure: .rejected(status: 409)) == nil)
        #expect(store.complete(record.id, attempt: nil, failure: .interrupted) == nil)
        #expect(store.record(record.id)?.status == .uploaded)
    }

    @Test func aSupersededAttemptsFailureIsIgnoredButItsSuccessCounts() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let record = try saved(store, .pending, attempts: 2)

        #expect(store.complete(record.id, attempt: 1, failure: .interrupted) == nil)
        #expect(store.record(record.id)?.status == .pending)
        #expect(FileManager.default.fileExists(atPath: store.bodyFile(for: record.id).path))

        #expect(store.complete(record.id, attempt: 1, failure: nil)?.status == .uploaded)
        #expect(!FileManager.default.fileExists(atPath: store.bodyFile(for: record.id).path))
    }

    @Test func theLatestAttemptsFailureIsRecorded() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let record = try saved(store, .pending, attempts: 2)

        #expect(store.complete(record.id, attempt: 2, failure: .rejected(status: 503))?.status
            == .failed(.rejected(status: 503)))
    }

    @Test func retainedBodiesAreCappedOldestFailureFirst() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let oldest = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 30, bodyBytes: 600)
        let older = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 20, bodyBytes: 600)
        let newest = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 10, bodyBytes: 600)
        let pending = try saved(store, .pending, minutesAgo: 40, bodyBytes: 600)

        store.prune(bodyLimit: 1300)

        #expect(Set(store.all().map(\.id)) == [older.id, newest.id, pending.id])
        #expect(!FileManager.default.fileExists(atPath: store.bodyFile(for: oldest.id).path))
    }

    @Test func bodiesFromEarlierBuildsAreCleanedUp() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let legacy = directory.appending(path: "upload-\(UUID().uuidString).multipart")
        let recent = directory.appending(path: "upload-\(UUID().uuidString).multipart")
        let unrelated = directory.appending(path: "notes.multipart")
        for file in [legacy, recent, unrelated] {
            try Data("--x--".utf8).write(to: file)
        }
        for file in [legacy, unrelated] {
            try FileManager.default.setAttributes(
                [.modificationDate: Date.now.addingTimeInterval(-2 * 24 * 60 * 60)], ofItemAtPath: file.path)
        }

        store.prune()

        #expect(!FileManager.default.fileExists(atPath: legacy.path))
        #expect(FileManager.default.fileExists(atPath: recent.path), "an old build's task may still be sending it")
        #expect(FileManager.default.fileExists(atPath: unrelated.path))
    }

    @Test func theMostRecentFailureIsKeptEvenWhenItAloneExceedsTheCap() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let older = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 20, bodyBytes: 600)
        let newest = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 10, bodyBytes: 600)

        store.prune(bodyLimit: 500)

        #expect(store.all().map(\.id) == [newest.id])
        #expect(store.record(older.id) == nil)
    }

    @Test func pruningLeavesAShareAnotherProcessIsRetrying() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let retrying = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 8 * 24 * 60)
        let other = ShareUploadStore(directory: directory)
        #expect(other.claim(retrying.id))

        store.prune()

        #expect(store.record(retrying.id) != nil)
        #expect(FileManager.default.fileExists(atPath: store.bodyFile(for: retrying.id).path))
        other.unclaim(retrying.id)
        store.prune()
        #expect(store.record(retrying.id) == nil)
        let leftovers = try FileManager.default.contentsOfDirectory(
            atPath: directory.appending(path: "share-uploads").path)
        #expect(leftovers.isEmpty, "removing a record also forgets its claim")
    }

    @Test func pruningAheadOfTimeNeverTakesOverALiveClaim() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let retrying = try saved(store, .failed(.rejected(status: 503)))
        let other = ShareUploadStore(directory: directory)
        #expect(other.claim(retrying.id))

        store.prune(now: .now.addingTimeInterval(30 * 24 * 60 * 60))

        #expect(store.record(retrying.id) != nil, "expired by that date, but another process holds it right now")
        other.unclaim(retrying.id)
    }

    @Test func pruningLeavesAShareThatChangedSinceItWasListed() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        var record = try saved(store, .failed(.rejected(status: 503)), minutesAgo: 8 * 24 * 60)
        record.status = .pending
        record.updatedAt = .now
        try store.save(record)

        store.prune()

        #expect(store.record(record.id)?.status == .pending)
    }

    @Test func aClaimHeldThroughALongWaitIsRenewed() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let retrying = ShareUploadStore(directory: directory)
        let id = UUID()
        #expect(retrying.claim(id))
        try FileManager.default.setAttributes(
            [.modificationDate: Date.now.addingTimeInterval(-2 * ShareUploadStore.claimTimeout)],
            ofItemAtPath: directory.appending(path: "share-uploads/\(id.uuidString).claim.1").path)

        #expect(retrying.renewClaim(id))

        #expect(!ShareUploadStore(directory: directory).claim(id), "the renewed claim is not taken over")
        #expect(!ShareUploadStore(directory: directory).renewClaim(id), "only the holder renews")
    }

    @Test func aLateUnclaimDoesNotReleaseAClaimTakenOverSince() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let slow = ShareUploadStore(directory: directory)
        let other = ShareUploadStore(directory: directory)
        let id = UUID()
        #expect(slow.claim(id))
        #expect(!other.claim(id))

        #expect(other.claim(id, now: .now.addingTimeInterval(2 * ShareUploadStore.claimTimeout)))
        slow.unclaim(id)

        #expect(!ShareUploadStore(directory: directory).claim(id))
        other.unclaim(id)
        #expect(ShareUploadStore(directory: directory).claim(id))
    }
}

@Suite(.timeLimit(.minutes(1))) struct BackgroundUploaderTests {
    @Test func acceptedUploadIsRecordedAndItsBodyDeleted() async throws {
        let harness = ShareUploadHarness([.status(201)])
        defer { harness.tearDown() }
        let vaultID = UUID()

        try await harness.uploader.enqueue(pdf(), vaultID: vaultID)
        let record = try #require(await harness.nextOutcome())

        #expect(record.status == .uploaded)
        #expect(record.vaultID == vaultID)
        #expect(!harness.bodyExists(record))
        #expect(harness.uploader.store.record(record.id)?.status == .uploaded)
        let request = try #require(ShareUploadProtocol.requests(to: harness.host).first)
        #expect(request.url?.path() == "/api/documents/upload")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer stale")
        #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
    }

    @Test func unauthorizedUploadIsRetainedAndRetriedWithARefreshedToken() async throws {
        let harness = ShareUploadHarness([.status(401), .status(201)])
        defer { harness.tearDown() }

        try await harness.uploader.enqueue(pdf())
        let failed = try #require(await harness.nextOutcome())
        #expect(failed.status == .failed(.unauthorized))
        #expect(failed.canRetry)
        #expect(harness.bodyExists(failed))

        try await harness.uploader.retry(failed.id)
        let retried = try #require(await harness.nextOutcome())

        #expect(retried.id == failed.id)
        #expect(retried.status == .uploaded)
        #expect(retried.attempts == 2)
        #expect(!harness.bodyExists(retried))
        #expect(harness.tokens.rejected == ["stale"])
        let requests = ShareUploadProtocol.requests(to: harness.host)
        #expect(requests.map { $0.value(forHTTPHeaderField: "Authorization") } == ["Bearer stale", "Bearer fresh"])
        #expect(
            requests.map { $0.value(forHTTPHeaderField: "Idempotency-Key") }
                == Array(repeating: failed.id.uuidString.lowercased(), count: 2))
        #expect(Set(requests.compactMap { $0.value(forHTTPHeaderField: "Content-Type") }).count == 1)
    }

    @Test func serverErrorIsRetainedAndRetried() async throws {
        let harness = ShareUploadHarness([.status(500), .status(201)])
        defer { harness.tearDown() }

        try await harness.uploader.enqueue(pdf())
        let failed = try #require(await harness.nextOutcome())
        #expect(failed.status == .failed(.rejected(status: 500)))
        #expect(harness.bodyExists(failed))

        try await harness.uploader.retry(failed.id)
        let retried = try #require(await harness.nextOutcome())

        #expect(retried.status == .uploaded)
        #expect(!harness.bodyExists(retried))
        #expect(harness.tokens.rejected.isEmpty)
    }

    @Test func transportFailureIsRetainedAndRetried() async throws {
        let harness = ShareUploadHarness([.transport(.notConnectedToInternet), .status(201)])
        defer { harness.tearDown() }

        try await harness.uploader.enqueue(pdf())
        let failed = try #require(await harness.nextOutcome())
        guard case .failed(.transport) = failed.status else {
            Issue.record("Expected a transport failure, got \(failed.status)")
            return
        }
        #expect(failed.canRetry)
        #expect(harness.bodyExists(failed))

        try await harness.uploader.retry(failed.id)
        #expect(await harness.nextOutcome()?.status == .uploaded)
    }

    @Test func refusedFileIsKeptVisibleButNotRetried() async throws {
        let harness = ShareUploadHarness([.status(415)])
        defer { harness.tearDown() }

        try await harness.uploader.enqueue(pdf())
        let failed = try #require(await harness.nextOutcome())
        #expect(failed.status == .failed(.rejected(status: 415)))
        #expect(!failed.canRetry)
        #expect(!harness.bodyExists(failed), "a body the server refused is not kept")

        await #expect(throws: ShareRetryError.unavailable) { try await harness.uploader.retry(failed.id) }
        #expect(ShareUploadProtocol.requests(to: harness.host).count == 1)

        harness.uploader.discard(failed.id)
        #expect(harness.uploader.retainedUploads().isEmpty)
        #expect(!harness.bodyExists(failed))
    }

    @Test func outcomeOutlivesTheUploaderThatStartedIt() async throws {
        let harness = ShareUploadHarness([.status(503)])
        defer { harness.tearDown() }

        try await harness.uploader.enqueue(pdf("bill.pdf"))
        let failed = try #require(await harness.nextOutcome())

        let relaunched = ShareUploadHarness.uploader(
            host: harness.host, tokens: harness.tokens, directory: harness.directory)
        defer { relaunched.invalidate() }
        let retained = relaunched.retainedUploads()
        #expect(retained.map(\.id) == [failed.id])
        #expect(retained.first?.filename == "bill.pdf")
        #expect(retained.first?.status == .failed(.rejected(status: 503)))
    }

    @Test func acceptedUploadsAgeOutAfterADayAndFailedOnesAfterAWeek() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-share-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        func record(_ status: ShareUploadRecord.Status, daysAgo: Double) throws -> ShareUploadRecord {
            let record = ShareUploadRecord(
                id: UUID(), filename: "a.pdf", contentType: "multipart/form-data", status: status, attempts: 1,
                updatedAt: .now.addingTimeInterval(-daysAgo * 24 * 60 * 60)
            )
            try store.save(record)
            try Data("body".utf8).write(to: store.bodyFile(for: record.id))
            return record
        }
        _ = try record(.uploaded, daysAgo: 2)
        let recentUpload = try record(.uploaded, daysAgo: 0.5)
        let recentFailure = try record(.failed(.rejected(status: 500)), daysAgo: 6)
        let oldFailure = try record(.failed(.rejected(status: 500)), daysAgo: 8)
        let pending = try record(.pending, daysAgo: 8)

        store.prune()

        #expect(Set(store.all().map(\.id)) == [recentUpload.id, recentFailure.id, pending.id])
        #expect(!FileManager.default.fileExists(atPath: store.bodyFile(for: oldFailure.id).path))
    }

    @Test func pendingUploadWithoutATaskBecomesRetryable() async throws {
        let harness = ShareUploadHarness([.status(201)])
        defer { harness.tearDown() }
        let record = try harness.savePending(sessionIdentifier: harness.uploader.identifier, minutesAgo: 10)
        let recent = try harness.savePending(sessionIdentifier: harness.uploader.identifier, minutesAgo: 0)
        let otherSession = try harness.savePending(sessionIdentifier: "elsewhere", minutesAgo: 10)

        let running = await harness.uploader.reconcile(interruptedBefore: .now.addingTimeInterval(-60))

        #expect(running == 0)
        let interrupted = try #require(harness.uploader.store.record(record.id))
        #expect(interrupted.status == .failed(.interrupted))
        #expect(interrupted.canRetry)
        #expect(harness.uploader.store.record(recent.id)?.status == .pending)
        #expect(harness.uploader.store.record(otherSession.id)?.status == .pending)

        try await harness.uploader.retry(record.id)
        #expect(await harness.nextOutcome()?.status == .failed(.interrupted), "reconcile reports the change")
        #expect(await harness.nextOutcome()?.status == .uploaded)
    }

    @Test func aShareWhoseKeyWasUsedForAnotherFileIsNotOfferedAgain() async throws {
        let harness = ShareUploadHarness([.json(409, #"{"error":"used","code":"idempotency_key_mismatch"}"#)])
        defer { harness.tearDown() }
        try await harness.uploader.enqueue(pdf())

        let failed = try #require(await harness.nextOutcome())

        #expect(failed.status == .failed(.keyReused))
        #expect(!failed.canRetry)
        #expect(!harness.bodyExists(failed))
        #expect(harness.uploader.heldAnswers == 0, "a finished task's answer is dropped")
    }

    @Test func refusalsHeldWhenTheSessionEndsAreDropped() {
        let harness = ShareUploadHarness([])
        defer { harness.tearDown() }
        harness.uploader.hold(Data("{}".utf8), task: 1, status: 409)
        harness.uploader.hold(Data("{}".utf8), task: 2, status: 409)
        #expect(harness.uploader.heldAnswers == 2)

        harness.uploader.urlSession(URLSession.shared, didBecomeInvalidWithError: nil)

        #expect(harness.uploader.heldAnswers == 0)
    }

    @Test func aRetryHeldByAnotherProcessIsRefused() async throws {
        let harness = ShareUploadHarness([.status(503), .status(201)])
        defer { harness.tearDown() }
        try await harness.uploader.enqueue(pdf())
        let failed = try #require(await harness.nextOutcome())

        #expect(harness.uploader.store.claim(failed.id))
        await #expect(throws: ShareRetryError.inProgress) { try await harness.uploader.retry(failed.id) }
        harness.uploader.store.unclaim(failed.id)

        try await harness.uploader.retry(failed.id)
        #expect(await harness.nextOutcome()?.status == .uploaded)
        #expect(ShareUploadProtocol.requests(to: harness.host).count == 2)
    }

    @Test func aShareForAnotherServerIsNotRetriedHere() async throws {
        let harness = ShareUploadHarness([.status(503)])
        defer { harness.tearDown() }
        try await harness.uploader.enqueue(pdf())
        var failed = try #require(await harness.nextOutcome())
        failed.serverURL = URL(string: "https://other.example.com")
        try harness.uploader.store.save(failed)

        await #expect(throws: ShareRetryError.otherServer) { try await harness.uploader.retry(failed.id) }
        #expect(ShareUploadProtocol.requests(to: harness.host).count == 1)
    }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct ShareSessionCoordinatorTests {
    @Test func relaunchedSessionEventsCompleteOnlyAfterTheSessionFinishes() async throws {
        let harness = ShareUploadHarness([])
        defer { harness.tearDown() }
        var attached: [String] = []
        let coordinator = ShareSessionCoordinator(store: harness.uploader.store) { identifier in
            attached.append(identifier)
            return harness.uploader
        }
        var completed = 0
        let identifier = BackgroundUploader.identifierPrefix + "relaunch"

        #expect(!coordinator.handleEvents(for: "com.example.other", completion: { completed += 1 }))
        #expect(coordinator.handleEvents(for: identifier, completion: { completed += 1 }))
        #expect(attached == [identifier])
        #expect(completed == 0)

        harness.uploader.onEventsFinished?()
        for _ in 0..<50 where completed == 0 {
            await Task.yield()
        }

        #expect(completed == 1)
    }

    @Test func openingTheAppFailsStalePendingSharesWithoutATask() async throws {
        let harness = ShareUploadHarness([])
        defer { harness.tearDown() }
        let identifier = BackgroundUploader.identifierPrefix + "gone"
        let stale = try harness.savePending(sessionIdentifier: identifier, minutesAgo: 10)
        let orphan = try harness.savePending(sessionIdentifier: nil, minutesAgo: 10)
        let fresh = try harness.savePending(sessionIdentifier: identifier, minutesAgo: 0)
        let coordinator = ShareSessionCoordinator(store: harness.uploader.store) { identifier in
            ShareUploadHarness.uploader(
                host: harness.host, tokens: harness.tokens, directory: harness.directory, identifier: identifier)
        }
        var changed: [UUID] = []
        coordinator.onChange = { changed.append($0.id) }

        await coordinator.recoverInterrupted()

        let store = harness.uploader.store
        #expect(store.record(stale.id)?.status == .failed(.interrupted))
        #expect(store.record(orphan.id)?.status == .failed(.interrupted))
        #expect(store.record(fresh.id)?.status == .pending)
        for _ in 0..<50 where changed.count < 2 {
            await Task.yield()
        }
        #expect(Set(changed) == [stale.id, orphan.id])
    }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct ShareSessionOwnershipTests {
    @Test func aShareSheetFailsOrphansButLeavesSessionsToTheApp() async throws {
        let harness = ShareUploadHarness([])
        defer { harness.tearDown() }
        let running = try harness.savePending(
            sessionIdentifier: BackgroundUploader.identifierPrefix + "app", minutesAgo: 10)
        let orphan = try harness.savePending(sessionIdentifier: nil, minutesAgo: 10)
        var attached: [String] = []
        let coordinator = ShareSessionCoordinator(store: harness.uploader.store) { identifier in
            attached.append(identifier)
            return ShareUploadHarness.uploader(
                host: harness.host, tokens: harness.tokens, directory: harness.directory, identifier: identifier)
        }

        await coordinator.recoverInterrupted(reattachingSessions: false)

        #expect(attached.isEmpty, "opening another process's session would hide its running tasks")
        let store = harness.uploader.store
        #expect(store.record(running.id)?.status == .pending)
        #expect(store.record(orphan.id)?.status == .failed(.interrupted))
    }
}

@MainActor @Suite(.timeLimit(.minutes(1))) struct ShareViewModelBackgroundTests {
    @Test func backgroundShareIsQueuedThenItsFailureShownAndRetried() async throws {
        let harness = ShareUploadHarness([.status(500), .status(201)])
        defer { harness.tearDown() }
        let model = ShareViewModel(files: [pdf()], service: MockFolioService(), background: harness.uploader)

        await model.upload()
        #expect(model.state == .queued)
        let failed = try #require(await harness.nextOutcome())
        model.refreshRetained()
        #expect(model.retained.map(\.status) == [.failed(.rejected(status: 500))])

        await model.retry(failed.id)
        #expect(model.retryError == nil)
        #expect(await harness.nextOutcome()?.status == .uploaded)
        model.refreshRetained()

        #expect(model.retained.map(\.id) == [failed.id])
        #expect(model.retained.first?.status == .uploaded)
        #expect(ShareUploadProtocol.requests(to: harness.host).count == 2)
    }

    @Test func nextShareSheetListsEarlierFailuresButNotAcceptedUploads() async throws {
        let harness = ShareUploadHarness([.status(401), .status(201)])
        defer { harness.tearDown() }
        try await harness.uploader.enqueue(pdf("rejected.pdf"))
        _ = await harness.nextOutcome()
        try await harness.uploader.enqueue(pdf("accepted.pdf"))
        _ = await harness.nextOutcome()

        let model = ShareViewModel(files: [pdf()], service: MockFolioService(), background: harness.uploader)

        #expect(model.retained.map(\.filename) == ["rejected.pdf"])
        #expect(model.retained.first?.status == .failed(.unauthorized))
    }

    @Test func refusedRetryIsReported() async throws {
        let harness = ShareUploadHarness([.status(415)])
        defer { harness.tearDown() }
        try await harness.uploader.enqueue(pdf())
        let failed = try #require(await harness.nextOutcome())
        let model = ShareViewModel(files: [pdf()], service: MockFolioService(), background: harness.uploader)

        await model.retry(failed.id)

        #expect(model.retryError == ShareRetryError.unavailable.localizedDescription)
        #expect(model.retained.first?.status == .failed(.rejected(status: 415)))
    }
}

struct RefusalAnswersTests {
    private let reused = Data(#"{"error":"used","code":"idempotency_key_mismatch"}"#.utf8)

    @Test func onlyARefusalsAnswerIsKept() {
        var answers = RefusalAnswers()

        answers.append(Data(#"{"id":"document"}"#.utf8), task: 1, status: 201)
        answers.append(reused, task: 2, status: 409)
        answers.append(reused, task: 3, status: nil)

        #expect(answers.count == 1)
        let accepted = answers.take(1)
        let refused = answers.take(2)
        #expect(accepted == nil)
        #expect(refused == reused)
        #expect(answers.count == 0, "a taken answer is gone")
    }

    @Test func manyRefusalsHeldTogetherAllKeepTheirCode() throws {
        var answers = RefusalAnswers()
        let tasks = 0..<300

        for task in tasks {
            answers.append(reused, task: task, status: 409)
        }

        let url = try #require(URL(string: "https://folio.example.com"))
        let response = try #require(HTTPURLResponse(url: url, statusCode: 409, httpVersion: nil, headerFields: nil))
        for task in tasks {
            let body = answers.take(task)
            let failure = ShareUploadFailure.classify(response: response, error: nil, body: body)
            #expect(failure == .keyReused, "task \(task)")
        }
    }

    @Test func refusalsWhoseTasksNeverFinishAreBounded() {
        var answers = RefusalAnswers()

        for task in 0..<(RefusalAnswers.taskLimit + 10) {
            answers.append(reused, task: task, status: 409)
        }
        answers.append(Data(count: 2 * RefusalAnswers.byteLimit), task: 0, status: 409)

        #expect(answers.count == RefusalAnswers.taskLimit)
        let first = answers.take(0)
        #expect(first?.count == RefusalAnswers.byteLimit)
        answers.removeAll()
        #expect(answers.count == 0)
    }
}
