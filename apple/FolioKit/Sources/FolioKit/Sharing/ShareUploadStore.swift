import Foundation

/// Why the server did not accept a background share.
public enum ShareUploadFailure: Codable, Equatable, Sendable {
    case unauthorized
    case rejected(status: Int)
    case transport(String)

    /// Classifies a finished upload task. Returns nil when the server accepted the file.
    public static func classify(response: URLResponse?, error: (any Error)?) -> ShareUploadFailure? {
        if let error {
            return .transport(error.localizedDescription)
        }
        guard let status = (response as? HTTPURLResponse)?.statusCode else {
            return .transport(FolioError.unreadableResponse.localizedDescription)
        }
        switch status {
        case 200..<300: return nil
        case 401: return .unauthorized
        default: return .rejected(status: status)
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

    public let id: UUID
    public var filename: String
    public var title: String?
    public var vaultID: UUID?
    public var contentType: String
    public var status: Status
    public var attempts: Int
    public var updatedAt: Date

    public var displayName: String { title ?? filename }

    public var canRetry: Bool {
        if case let .failed(failure) = status { return failure.isRetryable }
        return false
    }
}

/// Upload records and their bodies, one JSON file per upload so the app and the share extension can both
/// write to the shared container without overwriting each other's entries.
public final class ShareUploadStore: @unchecked Sendable {
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
        lock.withLock {
            let files = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil))
                ?? []
            return files.filter { $0.pathExtension == "json" }.compactMap(read)
                .sorted { $0.updatedAt > $1.updatedAt }
        }
    }

    /// Records the outcome of a finished upload. A body is only deleted once the server accepted it.
    @discardableResult
    public func complete(_ id: UUID, failure: ShareUploadFailure?, at date: Date = .now) -> ShareUploadRecord? {
        lock.withLock {
            guard var record = read(recordFile(for: id)) else { return nil }
            record.status = failure.map(ShareUploadRecord.Status.failed) ?? .uploaded
            record.updatedAt = date
            if failure == nil {
                try? FileManager.default.removeItem(at: bodyFile(for: id))
            }
            try? JSONEncoder().encode(record).write(to: recordFile(for: id), options: .atomic)
            return record
        }
    }

    public func remove(_ id: UUID) {
        lock.withLock {
            try? FileManager.default.removeItem(at: bodyFile(for: id))
            try? FileManager.default.removeItem(at: recordFile(for: id))
        }
    }

    /// Forgets uploads the server accepted before `date`; their outcome has been shown long enough.
    public func pruneUploaded(before date: Date) {
        for record in all() where record.status == .uploaded && record.updatedAt < date {
            remove(record.id)
        }
    }

    private func read(_ url: URL) -> ShareUploadRecord? {
        (try? Data(contentsOf: url)).flatMap { try? JSONDecoder().decode(ShareUploadRecord.self, from: $0) }
    }
}
