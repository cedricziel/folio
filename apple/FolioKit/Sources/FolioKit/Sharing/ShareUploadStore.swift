import Foundation

/// Why the server did not accept a background share.
public enum ShareUploadFailure: Codable, Equatable, Sendable {
    case unauthorized
    case rejected(status: Int)
    case transport(String)
    /// The share's key was already used for a different file, so sending it again never succeeds.
    case keyReused

    /// The upload's task disappeared without reporting back, e.g. because the app was force-quit.
    public static let interrupted = ShareUploadFailure.transport("The upload stopped before Folio answered.")

    /// Classifies a finished upload task from its response and, when it was delivered, the start of its body.
    /// Returns nil when the server accepted the file.
    public static func classify(response: URLResponse?, error: (any Error)?, body: Data? = nil)
        -> ShareUploadFailure?
    {
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
        case 409?:
            if let http = response as? HTTPURLResponse, KeyedUploadConflict(response: http, body: body) == .keyReused {
                return .keyReused
            }
            return .rejected(status: 409)
        case let status?: return .rejected(status: status)
        }
    }

    /// Server-side refusals of the file itself (bad type, too large) fail the same way again.
    public var isRetryable: Bool {
        switch self {
        case .unauthorized, .transport: true
        case .keyReused: false
        // 409: an earlier attempt with the same Idempotency-Key is still storing the file.
        case let .rejected(status): [408, 409, 429].contains(status) || status >= 500
        }
    }

    public var message: String {
        switch self {
        case .unauthorized: "Folio no longer accepts your sign-in. Open Folio to sign in again, then retry."
        case .rejected(409): "Folio is still filing an earlier attempt of this share. Retry in a moment."
        case .rejected(410):
            "Folio filed this share, but the document has been deleted since. Share the file again to file it anew."
        case let .rejected(status) where status >= 500: "The server failed while filing it (status \(status))."
        case let .rejected(status): "The server refused it (status \(status))."
        case let .transport(message): message
        case .keyReused: "Folio refused it because its upload key was used for a different file. Share the file again."
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
    /// Retained bodies of failed shares beyond this many bytes are dropped, oldest first.
    public static let retainedBodyLimit = 512 * 1024 * 1024

    public let directory: URL
    private let lock = NSLock()
    private var claims: [UUID: Int] = [:]

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
    ///
    /// Every attempt sends the same `Idempotency-Key`, so an accepted attempt means the document exists, whichever
    /// attempt it was. A failure counts only for the latest attempt (`attempt` nil for tasks started before
    /// attempts were tracked), and nothing downgrades an accepted upload. Returns nil when nothing changed.
    @discardableResult
    public func complete(_ id: UUID, attempt: Int? = nil, failure: ShareUploadFailure?, at date: Date = .now)
        -> ShareUploadRecord?
    {
        lock.withLock {
            guard var record = read(recordFile(for: id)), record.status != .uploaded else { return nil }
            if failure != nil, let attempt, attempt != record.attempts {
                return nil
            }
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
            unlockedRemove(id)
            claims[id] = nil
        }
        lease(for: id).removeAll()
    }

    /// Forgets accepted uploads after a day and failed ones after a week, then the oldest failed ones while their
    /// bodies take more than `bodyLimit` bytes, always keeping the most recent failure. Also deletes bodies left
    /// behind by builds that kept them outside this store.
    ///
    /// A record is only removed while this store holds its retry claim and it is still in the state it was chosen
    /// for, so a share another process is retrying keeps its body.
    public func prune(now: Date = .now, bodyLimit: Int = ShareUploadStore.retainedBodyLimit) {
        func expired(_ record: ShareUploadRecord) -> Bool {
            switch record.status {
            case .uploaded: record.updatedAt < now.addingTimeInterval(-Self.uploadedRetention)
            case .failed: record.updatedAt < now.addingTimeInterval(-Self.failedRetention)
            case .pending: false
            }
        }
        var failed: [ShareUploadRecord] = []
        for record in all() {
            if expired(record) {
                removeIfUnclaimed(record.id, where: expired)
            } else if case .failed = record.status {
                failed.append(record)
            }
        }
        var retained = failed.reduce(0) { $0 + bodySize($1.id) }
        for record in failed.dropFirst().reversed() where retained > bodyLimit {
            let size = bodySize(record.id)
            if removeIfUnclaimed(record.id, where: { $0 == record }) {
                retained -= size
            }
        }
        removeLegacyBodies(before: now.addingTimeInterval(-Self.uploadedRetention))
    }

    /// Claims with the store's own clock: `prune`'s `now` only decides what has expired, and a date ahead of it
    /// must never count a live claim as abandoned.
    @discardableResult
    private func removeIfUnclaimed(_ id: UUID, where unchanged: (ShareUploadRecord) -> Bool) -> Bool {
        guard claim(id) else { return false }
        let removed = lock.withLock {
            guard let record = read(recordFile(for: id)), unchanged(record) else { return false }
            unlockedRemove(id)
            return true
        }
        if removed {
            remove(id)
        } else {
            unclaim(id)
        }
        return removed
    }

    private func unlockedRemove(_ id: UUID) {
        try? FileManager.default.removeItem(at: bodyFile(for: id))
        try? FileManager.default.removeItem(at: recordFile(for: id))
    }

    /// Reserves a record for one retry across the app and its extensions. Returns false while another process
    /// holds it.
    public func claim(_ id: UUID, now: Date = .now) -> Bool {
        guard let token = lease(for: id).acquire(now: now) else { return false }
        lock.withLock { claims[id] = token }
        return true
    }

    /// Keeps this store's claim on `id` from being taken over while a retry waits, e.g. for a token refresh. False
    /// when it no longer holds the claim.
    public func renewClaim(_ id: UUID, now: Date = .now) -> Bool {
        guard let token = lock.withLock({ claims[id] }) else { return false }
        return lease(for: id).renew(token, now: now)
    }

    /// Ends this store's claim on `id`, leaving a claim another process has taken over since in place.
    public func unclaim(_ id: UUID) {
        guard let token = lock.withLock({ claims.removeValue(forKey: id) }) else { return }
        lease(for: id).release(token)
    }

    private func lease(for id: UUID) -> FileLease {
        FileLease(file: claimFile(for: id), timeout: Self.claimTimeout)
    }

    private func bodySize(_ id: UUID) -> Int {
        (try? bodyFile(for: id).resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    }

    /// Builds before bodies were kept per record wrote `upload-<uuid>.multipart` next to this store's directory.
    private func removeLegacyBodies(before cutoff: Date) {
        let parent = directory.deletingLastPathComponent()
        let files =
            (try? FileManager.default.contentsOfDirectory(
                at: parent, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for file in files
        where file.lastPathComponent.hasPrefix("upload-") && file.pathExtension == "multipart" {
            let modified = try? file.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
            if let modified, modified < cutoff {
                try? FileManager.default.removeItem(at: file)
            }
        }
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
