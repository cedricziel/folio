import Foundation

/// The upload request a background share sends with its retained body.
enum ShareUploadRequest {
    static func request(
        serverURL: URL, contentType: String, token: String?, vaultID: UUID?, idempotencyKey: UUID
    ) -> URLRequest {
        var request = URLRequest(url: serverURL.appending(path: "api/documents/upload"))
        request.httpMethod = "POST"
        request.setValue(contentType, forHTTPHeaderField: "Content-Type")
        request.setValue(idempotencyKey.uuidString.lowercased(), forHTTPHeaderField: "Idempotency-Key")
        if let vaultID {
            request.setValue(vaultID.uuidString.lowercased(), forHTTPHeaderField: "X-Folio-Vault")
        }
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }
}

public enum ShareRetryError: Error, Equatable, LocalizedError {
    case unavailable
    /// Another sheet or the app is retrying the same share right now.
    case inProgress
    /// The share was sent to a server Folio is no longer connected to.
    case otherServer

    public var errorDescription: String? {
        switch self {
        case .unavailable: "This share can no longer be retried. Share the file again."
        case .inProgress: "This share is already being retried."
        case .otherServer: "This share was meant for another server. Discard it and share the file again."
        }
    }
}

public protocol BackgroundUploading: Sendable {
    func enqueue(_ file: UploadFile) async throws
    func enqueue(_ file: UploadFile, vaultID: UUID?) async throws
    /// Shares the server has not accepted yet, and recent outcomes, most recent first.
    func retainedUploads() -> [ShareUploadRecord]
    /// Sends a failed share's retained body again.
    func retry(_ id: UUID) async throws
    /// Forgets a share and deletes its retained body.
    func discard(_ id: UUID)
}

public extension BackgroundUploading {
    func enqueue(_ file: UploadFile, vaultID: UUID?) async throws {
        guard vaultID == nil else { throw FolioError.unreadableResponse }
        try await enqueue(file)
    }

    func retainedUploads() -> [ShareUploadRecord] { [] }
    func retry(_: UUID) async throws { throw ShareRetryError.unavailable }
    func discard(_: UUID) {}
}

/// Uploads through a background `URLSession`, so large files finish after the share sheet has closed.
///
/// Each upload is recorded in a `ShareUploadStore` before it starts. Its body stays on disk until the server
/// accepts it, so rejected, unauthorized and interrupted shares remain visible and can be retried. Every attempt
/// carries the record's id as `Idempotency-Key`, so a retry after a lost answer returns the first document.
public final class BackgroundUploader: NSObject, BackgroundUploading, URLSessionDataDelegate, @unchecked Sendable {
    public static let identifierPrefix = "com.cedricziel.folio.share."

    public static func makeIdentifier() -> String {
        identifierPrefix + UUID().uuidString
    }

    public let identifier: String
    private let serverURL: URL?
    private let tokens: (any BearerTokenProvider)?
    public let store: ShareUploadStore
    private let lock = NSLock()
    private var session: URLSession!
    private var changeHandler: (@Sendable (ShareUploadRecord) -> Void)?
    private var eventsFinishedHandler: (@Sendable () -> Void)?
    private var answers = RefusalAnswers()

    /// - Parameters:
    ///   - serverURL: Where new shares go; nil for an uploader that only collects the outcome of earlier ones.
    ///   - configuration: Defaults to a background configuration named `identifier`.
    public init(
        serverURL: URL?, tokens: (any BearerTokenProvider)?, identifier: String, directory: URL? = nil,
        configuration: URLSessionConfiguration? = nil
    ) {
        self.identifier = identifier
        self.serverURL = serverURL
        self.tokens = tokens
        store = ShareUploadStore(
            directory: directory ?? SharedConfiguration.container ?? FileManager.default.temporaryDirectory)
        super.init()
        let configuration =
            configuration
            ?? {
                let background = URLSessionConfiguration.background(withIdentifier: identifier)
                background.sharedContainerIdentifier = SharedConfiguration.appGroup
                background.sessionSendsLaunchEvents = true
                return background
            }()
        session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }

    /// Called on a background queue whenever an upload finishes, successfully or not.
    public var onChange: (@Sendable (ShareUploadRecord) -> Void)? {
        get { lock.withLock { changeHandler } }
        set { lock.withLock { changeHandler = newValue } }
    }

    /// Called once the system has delivered every event of a relaunched background session.
    public var onEventsFinished: (@Sendable () -> Void)? {
        get { lock.withLock { eventsFinishedHandler } }
        set { lock.withLock { eventsFinishedHandler = newValue } }
    }

    public func enqueue(_ file: UploadFile) async throws {
        try await enqueue(file, vaultID: nil)
    }

    public func enqueue(_ file: UploadFile, vaultID: UUID?) async throws {
        guard let serverURL else { throw ShareRetryError.otherServer }
        let token = try await tokens?.token()
        let form = MultipartForm(file: file)
        let record = ShareUploadRecord(
            id: UUID(), filename: file.filename, title: file.title, vaultID: vaultID,
            contentType: form.contentType, status: .pending, attempts: 1, updatedAt: .now,
            serverURL: serverURL, sessionIdentifier: identifier
        )
        try store.prepareDirectory()
        let body = store.bodyFile(for: record.id)
        try form.body.write(to: body)
        do {
            try store.save(record)
        } catch {
            try? FileManager.default.removeItem(at: body)
            throw error
        }
        start(record, serverURL: serverURL, token: token)
    }

    public func retainedUploads() -> [ShareUploadRecord] {
        store.prune()
        return store.all()
    }

    public func retry(_ id: UUID) async throws {
        guard store.claim(id) else { throw ShareRetryError.inProgress }
        // A token refresh can outlast the claim's timeout; renew it so no other process starts the same share.
        let renewal = Task { [store] in
            while (try? await Task.sleep(for: .seconds(ShareUploadStore.claimTimeout / 3))) != nil {
                _ = store.renewClaim(id)
            }
        }
        defer {
            renewal.cancel()
            store.unclaim(id)
        }
        guard var record = store.record(id), record.canRetry,
            FileManager.default.fileExists(atPath: store.bodyFile(for: id).path)
        else { throw ShareRetryError.unavailable }
        guard let serverURL, record.serverURL == nil || record.serverURL == serverURL else {
            throw ShareRetryError.otherServer
        }
        var token = try await tokens?.token()
        if record.status == .failed(.unauthorized), let tokens {
            token = try await tokens.refresh(rejecting: token)
            guard token != nil else { throw FolioError.signInRequired }
        }
        guard store.renewClaim(id) else { throw ShareRetryError.inProgress }
        record.status = .pending
        record.attempts += 1
        record.updatedAt = .now
        record.serverURL = serverURL
        record.sessionIdentifier = identifier
        try store.save(record)
        start(record, serverURL: serverURL, token: token)
    }

    public func discard(_ id: UUID) {
        store.remove(id)
    }

    /// Marks this session's pending uploads that no longer have a task, and have not changed since `before`, as
    /// interrupted, so they can be retried. Returns how many of its uploads are still running.
    @discardableResult
    public func reconcile(interruptedBefore before: Date) async -> Int {
        let live = Set(await session.allTasks.compactMap { $0.taskDescription.flatMap { ShareTask($0) }?.id })
        for record in store.markInterrupted(sessionIdentifier: identifier, live: live, before: before) {
            onChange?(record)
        }
        return live.count
    }

    /// Tears the session down once its tasks have finished.
    public func invalidate() {
        session.finishTasksAndInvalidate()
    }

    private func start(_ record: ShareUploadRecord, serverURL: URL, token: String?) {
        let request = ShareUploadRequest.request(
            serverURL: serverURL, contentType: record.contentType, token: token, vaultID: record.vaultID,
            idempotencyKey: record.id
        )
        let task = session.uploadTask(with: request, fromFile: store.bodyFile(for: record.id))
        task.taskDescription = ShareTask(id: record.id, attempt: record.attempts).description
        task.resume()
    }

    public func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        TelemetryRedirectGuard().urlSession(
            session, task: task, willPerformHTTPRedirection: response,
            newRequest: request, completionHandler: completionHandler
        )
    }

    public func urlSession(_: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        let answer = lock.withLock { answers.take(task.taskIdentifier) }
        guard let description = task.taskDescription else { return }
        guard let shareTask = ShareTask(description) else {
            // Tasks enqueued by earlier versions carry the path of their body instead of a record.
            try? FileManager.default.removeItem(atPath: description)
            return
        }
        let failure = ShareUploadFailure.classify(response: task.response, error: error, body: answer)
        if let record = store.complete(shareTask.id, attempt: shareTask.attempt, failure: failure) {
            onChange?(record)
        }
    }

    public func urlSession(_: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        hold(data, task: dataTask.taskIdentifier, status: (dataTask.response as? HTTPURLResponse)?.statusCode)
    }

    func hold(_ data: Data, task: Int, status: Int?) {
        lock.withLock { answers.append(data, task: task, status: status) }
    }

    public func urlSession(_: URLSession, didBecomeInvalidWithError _: (any Error)?) {
        lock.withLock { answers.removeAll() }
    }

    /// How many task answers are held right now.
    var heldAnswers: Int { lock.withLock { answers.count } }

    public func urlSessionDidFinishEvents(forBackgroundURLSession _: URLSession) {
        onEventsFinished?()
    }
}

/// Which record and attempt a background task uploads, kept in its `taskDescription` so the outcome finds them
/// after a relaunch.
struct ShareTask: Equatable, LosslessStringConvertible {
    let id: UUID
    /// Nil for tasks started before attempts were recorded.
    let attempt: Int?

    init(id: UUID, attempt: Int?) {
        self.id = id
        self.attempt = attempt
    }

    init?(_ description: String) {
        let parts = description.split(separator: " ", omittingEmptySubsequences: false)
        guard let id = parts.first.flatMap({ UUID(uuidString: String($0)) }) else { return nil }
        switch parts.count {
        case 1: self.init(id: id, attempt: nil)
        case 2:
            guard let attempt = Int(parts[1]) else { return nil }
            self.init(id: id, attempt: attempt)
        default: return nil
        }
    }

    var description: String {
        attempt.map { "\(id.uuidString) \($0)" } ?? id.uuidString
    }
}

/// The start of each refused upload's answer, kept until its task completes so the refusal can be classified.
///
/// Only a 409's body is ever read, so other answers (an accepted upload's document) are not kept, and the limit on
/// tasks only matters for refusals whose completion never arrives.
struct RefusalAnswers {
    static let byteLimit = 4096
    static let taskLimit = 1024

    private var answers: [Int: Data] = [:]

    var count: Int { answers.count }

    mutating func append(_ data: Data, task: Int, status: Int?) {
        guard status == 409 else { return }
        let received = answers[task]
        guard received != nil || answers.count < Self.taskLimit else { return }
        let start = received ?? Data()
        guard start.count < Self.byteLimit else { return }
        answers[task] = start + data.prefix(Self.byteLimit - start.count)
    }

    mutating func take(_ task: Int) -> Data? {
        answers.removeValue(forKey: task)
    }

    mutating func removeAll() {
        answers.removeAll()
    }
}
