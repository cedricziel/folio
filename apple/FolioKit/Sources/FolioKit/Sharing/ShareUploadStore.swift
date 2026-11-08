import Foundation

/// Why the server did not accept a background share.
public enum ShareUploadFailure: Codable, Equatable, Sendable {
    case unauthorized
    case rejected(status: Int)
    case transport(String)

    /// The upload's task disappeared without reporting back, e.g. because the app was force-quit.
    public static let interrupted = ShareUploadFailure.transport("The upload stopped before Folio answered.")

    /// Classifies a finished upload task. Returns nil when the server accepted the file.
    public static func classify(response: URLResponse?, error: (any Error)?) -> ShareUploadFailure? {
        let status = (response as? HTTPURLResponse)?.statusCode
        if let status, (200..<300).contains(status) {
            return nil
        }
        if let error {
            return .transport(error.localizedDescription)
        }
        switch status {
        case nil: return .transport(FolioError.unreadableResponse.localizedDescription)
        case 401?: return .unauthorized
        case let status?: return .rejected(status: status)
        }
    }

    /// Server-side refusals of the file itself (bad type, too large) fail the same way again.
    public var isRetryable: Bool {
        switch self {
        case .unauthorized, .transport: true
        case let .rejected(status): status == 408 || status == 429 || status >= 500
        }
    }

    public var message: String {
        switch self {
        case .unauthorized: "Folio no longer accepts your sign-in. Open Folio to sign in again, then retry."
        case let .rejected(status) where status >= 500: "The server failed while filing it (status \(status))."
        case let .rejected(status): "The server refused it (status \(status))."
        case let .transport(message): message
        }
    }
}

/// One shared file whose multipart body is kept on disk until the server has accepted it.
public struct ShareUploadRecord: Codable, Equatable, Identifiable, Sendable {
    public enum Status: Codable, Equatable, Sendable {
        case pending
        case uploaded
        case failed(ShareUploadFailure)
    }

    /// Also the upload's `Idempotency-Key`, so a retry after a lost answer cannot create a second document.
    public let id: UUID
    public var filename: String
    public var title: String?
    public var vaultID: UUID?
    public var contentType: String
    public var status: Status
    public var attempts: Int
    public var updatedAt: Date
    /// The server the share was sent to; a retry goes to the same one.
    public var serverURL: URL?
    /// The background session running the latest attempt, so a relaunched app can ask whether it still runs.
    public var sessionIdentifier: String?

    public var displayName: String { title ?? filename }

    public var canRetry: Bool {
        if case let .failed(failure) = status { return failure.isRetryable }
        return false
    }
}

/// Upload records and their bodies, one JSON file per upload so the app and the share extension can both
/// write to the shared container without overwriting each other's entries.
public final class ShareUploadStore: @unchecked Sendable {
    /// Accepted uploads stay listed this long, so the outcome of a retry can be seen.
    public static let uploadedRetention: TimeInterval = 24 * 60 * 60
    /// Failed uploads, and their bodies, are forgotten after this.
    public static let failedRetention: TimeInterval = 7 * 24 * 60 * 60
    /// A retry claim left behind by a process that died stops blocking the record after this.
    static let claimTimeout: TimeInterval = 60

    public let directory: URL
    private let lock = NSLock()

    public init(directory: URL) {
        self.directory = directory.appending(path: "share-uploads", directoryHint: .isDirectory)
    }

    public static func shared() -> ShareUploadStore {
        ShareUploadStore(directory: SharedConfiguration.container ?? FileManager.default.temporaryDirectory)
    }

    public func bodyFile(for id: UUID) -> URL {
        directory.appending(path: "\(id.uuidString).multipart")
    }

    private func recordFile(for id: UUID) -> URL {
        directory.appending(path: "\(id.uuidString).json")
    }

    private func claimFile(for id: UUID) -> URL {
        directory.appending(path: "\(id.uuidString).claim")
    }

    public func prepareDirectory() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    public func save(_ record: ShareUploadRecord) throws {
        try lock.withLock {
            try prepareDirectory()
            let data = try JSONEncoder().encode(record)
            try data.write(to: recordFile(for: record.id), options: .atomic)
        }
    }

    public func record(_ id: UUID) -> ShareUploadRecord? {
        lock.withLock { read(recordFile(for: id)) }
    }

    /// Every known upload, most recently changed first.
    public func all() -> [ShareUploadRecord] {
        lock.withLock { unlockedAll() }
    }

    /// Records the outcome of a finished upload. The body is kept only while the server may still accept it.
    @discardableResult
    public func complete(_ id: UUID, failure: ShareUploadFailure?, at date: Date = .now) -> ShareUploadRecord? {
        lock.withLock {
            guard var record = read(recordFile(for: id)) else { return nil }
            record.status = failure.map(ShareUploadRecord.Status.failed) ?? .uploaded
            record.updatedAt = date
            write(record)
            return record
        }
    }

    /// Fails pending uploads of `sessionIdentifier` that have no live task and have not changed since `before`.
    @discardableResult
    public func markInterrupted(sessionIdentifier: String?, live: Set<UUID>, before: Date, at date: Date = .now)
        -> [ShareUploadRecord]
    {
        lock.withLock {
            unlockedAll()
                .filter {
                    $0.status == .pending && $0.sessionIdentifier == sessionIdentifier && !live.contains($0.id)
                        && $0.updatedAt < before
                }
                .map { record in
                    var record = record
                    record.status = .failed(.interrupted)
                    record.updatedAt = date
                    write(record)
                    return record
                }
        }
    }

    public func remove(_ id: UUID) {
        lock.withLock {
            try? FileManager.default.removeItem(at: bodyFile(for: id))
            try? FileManager.default.removeItem(at: recordFile(for: id))
            try? FileManager.default.removeItem(at: claimFile(for: id))
        }
    }

    /// Forgets accepted uploads after a day and failed ones after a week.
    public func prune(now: Date = .now) {
        for record in all() {
            switch record.status {
            case .uploaded where record.updatedAt < now.addingTimeInterval(-Self.uploadedRetention),
                .failed where record.updatedAt < now.addingTimeInterval(-Self.failedRetention):
                remove(record.id)
            default:
                break
            }
        }
    }

    /// Reserves a record for one retry across the app and its extensions. Returns false while another process
    /// holds it.
    public func claim(_ id: UUID, now: Date = .now) -> Bool {
        let file = claimFile(for: id)
        if let created = try? file.resourceValues(forKeys: [.creationDateKey]).creationDate,
            created < now.addingTimeInterval(-Self.claimTimeout)
        {
            try? FileManager.default.removeItem(at: file)
        }
        do {
            try prepareDirectory()
            try Data().write(to: file, options: .withoutOverwriting)
            return true
        } catch {
            return false
        }
    }

    public func unclaim(_ id: UUID) {
        try? FileManager.default.removeItem(at: claimFile(for: id))
    }

    private func unlockedAll() -> [ShareUploadRecord] {
        let files =
            (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
        return files.filter { $0.pathExtension == "json" }.compactMap(read).sorted { $0.updatedAt > $1.updatedAt }
    }

    /// Saves `record` and deletes its body once nothing can send it again.
    private func write(_ record: ShareUploadRecord) {
        if record.status == .uploaded || (record.status != .pending && !record.canRetry) {
            try? FileManager.default.removeItem(at: bodyFile(for: record.id))
        }
        try? JSONEncoder().encode(record).write(to: recordFile(for: record.id), options: .atomic)
    }

    private func read(_ url: URL) -> ShareUploadRecord? {
        (try? Data(contentsOf: url)).flatMap { try? JSONDecoder().decode(ShareUploadRecord.self, from: $0) }
    }
}
