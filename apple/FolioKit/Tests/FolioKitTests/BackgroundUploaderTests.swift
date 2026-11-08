import Foundation
import Testing

@testable import FolioKit

/// Answers share uploads by host, so tests running in parallel never consume each other's replies.
final class ShareUploadProtocol: URLProtocol, @unchecked Sendable {
    enum Reply: Sendable {
        case status(Int)
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
            let response = HTTPURLResponse(
                url: request.url!, statusCode: status, httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data("{}".utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
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

    static func uploader(host: String, tokens: RotatingTokens, directory: URL) -> BackgroundUploader {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ShareUploadProtocol.self]
        return BackgroundUploader(
            serverURL: URL(string: "https://\(host)")!, tokens: tokens, identifier: "test",
            directory: directory, configuration: configuration
        )
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

    @Test func transportErrorsWinOverAnyResponse() {
        let failure = ShareUploadFailure.classify(response: response(200), error: URLError(.networkConnectionLost))
        guard case .transport = failure else {
            Issue.record("Expected a transport failure, got \(String(describing: failure))")
            return
        }
    }

    @Test func onlyRepeatableFailuresAreRetryable() {
        #expect(ShareUploadFailure.unauthorized.isRetryable)
        #expect(ShareUploadFailure.transport("offline").isRetryable)
        #expect(ShareUploadFailure.rejected(status: 503).isRetryable)
        #expect(ShareUploadFailure.rejected(status: 429).isRetryable)
        #expect(!ShareUploadFailure.rejected(status: 415).isRetryable)
        #expect(!ShareUploadFailure.rejected(status: 413).isRetryable)
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

    @Test func acceptedUploadsArePrunedAfterADay() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-share-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareUploadStore(directory: directory)
        let old = ShareUploadRecord(
            id: UUID(), filename: "old.pdf", contentType: "multipart/form-data", status: .uploaded,
            attempts: 1, updatedAt: .now.addingTimeInterval(-2 * 24 * 60 * 60)
        )
        let failed = ShareUploadRecord(
            id: UUID(), filename: "failed.pdf", contentType: "multipart/form-data",
            status: .failed(.rejected(status: 500)), attempts: 1, updatedAt: old.updatedAt
        )
        try store.save(old)
        try store.save(failed)

        store.pruneUploaded(before: .now.addingTimeInterval(-24 * 60 * 60))

        #expect(store.all().map(\.id) == [failed.id])
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
