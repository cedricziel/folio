import Foundation

/// A request plus the multipart body written to disk, which is what background sessions upload from.
public struct ShareUploadRequest {
    public let request: URLRequest
    public let bodyFile: URL

    public static func make(
        file: UploadFile, serverURL: URL, token: String?, directory: URL, vaultID: UUID? = nil
    ) throws
        -> ShareUploadRequest
    {
        let form = MultipartForm(file: file)
        let bodyFile = directory.appending(path: "upload-\(UUID().uuidString).multipart")
        try form.body.write(to: bodyFile)
        let request = request(serverURL: serverURL, contentType: form.contentType, token: token, vaultID: vaultID)
        return ShareUploadRequest(request: request, bodyFile: bodyFile)
    }

    static func request(serverURL: URL, contentType: String, token: String?, vaultID: UUID?) -> URLRequest {
        var request = URLRequest(url: serverURL.appending(path: "api/documents/upload"))
        request.httpMethod = "POST"
        request.setValue(contentType, forHTTPHeaderField: "Content-Type")
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

    public var errorDescription: String? {
        "This share can no longer be retried. Share the file again."
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
/// accepts it, so rejected, unauthorized and interrupted shares remain visible and can be retried.
public final class BackgroundUploader: NSObject, BackgroundUploading, URLSessionTaskDelegate, @unchecked Sendable {
    public static let identifierPrefix = "com.cedricziel.folio.share."

    public static func makeIdentifier() -> String {
        identifierPrefix + UUID().uuidString
    }

    private let serverURL: URL
    private let tokens: (any BearerTokenProvider)?
    public let store: ShareUploadStore
    private let lock = NSLock()
    private var session: URLSession!
    private var changeHandler: (@Sendable (ShareUploadRecord) -> Void)?
    private var eventsFinishedHandler: (@Sendable () -> Void)?

    /// - Parameter configuration: Defaults to a background configuration named `identifier`.
    public init(
        serverURL: URL, tokens: (any BearerTokenProvider)?, identifier: String, directory: URL? = nil,
        configuration: URLSessionConfiguration? = nil
    ) {
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
        let token = try await tokens?.token()
        let form = MultipartForm(file: file)
        let record = ShareUploadRecord(
            id: UUID(), filename: file.filename, title: file.title, vaultID: vaultID,
            contentType: form.contentType, status: .pending, attempts: 1, updatedAt: .now
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
        start(record, token: token)
    }

    public func retainedUploads() -> [ShareUploadRecord] {
        store.pruneUploaded(before: .now.addingTimeInterval(-24 * 60 * 60))
        return store.all()
    }

    public func retry(_ id: UUID) async throws {
        guard var record = store.record(id), record.canRetry,
            FileManager.default.fileExists(atPath: store.bodyFile(for: id).path)
        else { throw ShareRetryError.unavailable }
        var token = try await tokens?.token()
        if record.status == .failed(.unauthorized), let tokens {
            token = try await tokens.refresh(rejecting: token)
            guard token != nil else { throw FolioError.signInRequired }
        }
        record.status = .pending
        record.attempts += 1
        record.updatedAt = .now
        try store.save(record)
        start(record, token: token)
    }

    public func discard(_ id: UUID) {
        store.remove(id)
    }

    /// Tears the session down; for uploaders that use a foreground configuration.
    public func invalidate() {
        session.finishTasksAndInvalidate()
    }

    private func start(_ record: ShareUploadRecord, token: String?) {
        let request = ShareUploadRequest.request(
            serverURL: serverURL, contentType: record.contentType, token: token, vaultID: record.vaultID
        )
        let task = session.uploadTask(with: request, fromFile: store.bodyFile(for: record.id))
        task.taskDescription = record.id.uuidString
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
        guard let description = task.taskDescription else { return }
        guard let id = UUID(uuidString: description) else {
            // Tasks enqueued by earlier versions carry the path of their body instead of a record.
            try? FileManager.default.removeItem(atPath: description)
            return
        }
        let failure = ShareUploadFailure.classify(response: task.response, error: error)
        if let record = store.complete(id, failure: failure) {
            onChange?(record)
        }
    }

    public func urlSessionDidFinishEvents(forBackgroundURLSession _: URLSession) {
        onEventsFinished?()
    }
}
