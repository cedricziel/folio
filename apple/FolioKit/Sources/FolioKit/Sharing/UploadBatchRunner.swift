import CryptoKit
import Foundation

/// Where one file of a batch stands: the document the server created for it and the tags still to attach.
public struct UploadStep: Codable, Equatable, Sendable {
    public var documentID: Document.ID?
    public var remainingTags: [Tag.ID]
    /// When an upload ended without a definite answer. The server may have created the document anyway, so the
    /// next attempt looks for it before uploading the file again.
    public var uncertainSince: Date?
}

/// Progress of uploading a batch and tagging each new document, kept so a retry resumes where it stopped.
public struct UploadBatchProgress: Codable, Equatable, Sendable {
    public var steps: [UploadStep]
    public var updatedAt: Date

    public var uploaded: Int { steps.filter { $0.documentID != nil }.count }
    public var remainingTags: Int { steps.reduce(0) { $0 + $1.remainingTags.count } }
}

public protocol UploadProgressStoring: Sendable {
    func load(_ key: String) -> UploadBatchProgress?
    func save(_ progress: UploadBatchProgress, for key: String)
    func remove(_ key: String)
}

public final class InMemoryUploadProgressStore: UploadProgressStoring, @unchecked Sendable {
    private let lock = NSLock()
    private var entries: [String: UploadBatchProgress] = [:]

    public init() {}

    public func load(_ key: String) -> UploadBatchProgress? { lock.withLock { entries[key] } }
    public func save(_ progress: UploadBatchProgress, for key: String) { lock.withLock { entries[key] = progress } }
    public func remove(_ key: String) { lock.withLock { entries[key] = nil } }
}

/// Keeps progress in the App Group container, so running a Shortcut again or sharing the same files again resumes
/// the earlier attempt instead of creating its documents twice.
public final class FileUploadProgressStore: UploadProgressStoring, @unchecked Sendable {
    public static let shared = FileUploadProgressStore(
        directory: SharedConfiguration.container ?? FileManager.default.temporaryDirectory)

    private let directory: URL
    private let maxAge: TimeInterval
    private let lock = NSLock()

    /// - Parameter maxAge: Progress older than this is forgotten; the same files shared later are a new batch.
    public init(directory: URL, maxAge: TimeInterval = 24 * 60 * 60) {
        self.directory = directory.appending(path: "upload-progress", directoryHint: .isDirectory)
        self.maxAge = maxAge
    }

    public func load(_ key: String) -> UploadBatchProgress? {
        lock.withLock {
            let file = file(for: key)
            guard let data = try? Data(contentsOf: file),
                let progress = try? JSONDecoder().decode(UploadBatchProgress.self, from: data)
            else { return nil }
            guard progress.updatedAt > .now.addingTimeInterval(-maxAge) else {
                try? FileManager.default.removeItem(at: file)
                return nil
            }
            return progress
        }
    }

    public func save(_ progress: UploadBatchProgress, for key: String) {
        lock.withLock {
            try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try? JSONEncoder().encode(progress).write(to: file(for: key), options: .atomic)
        }
    }

    public func remove(_ key: String) {
        lock.withLock { try? FileManager.default.removeItem(at: file(for: key)) }
    }

    private func file(for key: String) -> URL {
        directory.appending(path: "\(key).json")
    }
}

/// Uploads files one at a time and attaches tags to each new document, recording every finished step. Running
/// the same batch again skips uploads and tags that already succeeded.
///
/// Tag assignment is idempotent on the server, so a tag whose answer was lost is simply sent again. An upload
/// whose answer was lost is not: before sending such a file again, the runner looks for a document with the same
/// original filename created since that attempt, and adopts it instead.
public struct UploadBatchRunner: Sendable {
    /// Allowed difference between this device's clock and the server's when looking for uncertain uploads.
    static let clockSkew: TimeInterval = 10 * 60

    private let service: any FolioService
    private let store: any UploadProgressStoring
    private let now: @Sendable () -> Date

    public init(
        service: any FolioService, store: any UploadProgressStoring, now: @escaping @Sendable () -> Date = { .now }
    ) {
        self.service = service
        self.store = store
        self.now = now
    }

    /// Identifies a batch by its server and vault (`scope`), the files' names, titles, types and contents, and
    /// the tags to attach.
    public static func key(files: [UploadFile], tagIDs: [Tag.ID], scope: String) -> String {
        var hasher = SHA256()
        func add(_ string: String) {
            hasher.update(data: Data(string.utf8))
            hasher.update(data: Data([0]))
        }
        add(scope)
        tagIDs.forEach { add($0.uuidString) }
        for file in files {
            add(file.filename)
            add(file.title ?? "")
            add(file.mimeType)
            add(SHA256.hash(data: file.data).map { String(format: "%02x", $0) }.joined())
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    public func progress(files: [UploadFile], tagIDs: [Tag.ID], scope: String) -> UploadBatchProgress? {
        store.load(Self.key(files: files, tagIDs: tagIDs, scope: scope))
    }

    /// Runs the unfinished steps of the batch and returns its documents in file order. On failure the progress
    /// so far is kept and the error is rethrown.
    public func run(_ files: [UploadFile], tagIDs: [Tag.ID], scope: String) async throws -> [Document] {
        let key = Self.key(files: files, tagIDs: tagIDs, scope: scope)
        var progress =
            store.load(key).flatMap { $0.steps.count == files.count ? $0 : nil }
            ?? UploadBatchProgress(
                steps: files.map { _ in UploadStep(remainingTags: tagIDs) }, updatedAt: now())
        var documents: [Document?] = Array(repeating: nil, count: files.count)

        func record() {
            progress.updatedAt = now()
            store.save(progress, for: key)
        }

        for index in files.indices {
            let file = files[index]
            if progress.steps[index].documentID == nil {
                if let since = progress.steps[index].uncertainSince {
                    let claimed = Set(progress.steps.compactMap(\.documentID))
                    if let found = try await existing(file, since: since, excluding: claimed) {
                        documents[index] = found
                        progress.steps[index].documentID = found.id
                    }
                    progress.steps[index].uncertainSince = nil
                    record()
                }
            }
            if progress.steps[index].documentID == nil {
                let started = now()
                do {
                    let document = try await service.upload(file)
                    documents[index] = document
                    progress.steps[index].documentID = document.id
                    record()
                } catch {
                    if Self.isUncertain(error) {
                        progress.steps[index].uncertainSince = started
                    }
                    record()
                    throw error
                }
            }
            guard let documentID = progress.steps[index].documentID else { continue }
            while let tagID = progress.steps[index].remainingTags.first {
                do {
                    try await service.addTag(tagID, to: documentID)
                } catch {
                    record()
                    throw error
                }
                progress.steps[index].remainingTags.removeFirst()
                record()
            }
        }
        store.remove(key)

        var result: [Document] = []
        for index in files.indices {
            if let document = documents[index] {
                result.append(document)
            } else if let id = progress.steps[index].documentID {
                result.append(await resumed(id, file: files[index]))
            }
        }
        return result
    }

    /// An answer from the server, or a request that never left, means nothing was created.
    static func isUncertain(_ error: any Error) -> Bool {
        switch error as? FolioError {
        case .server?, .signInRequired?, .signInFailed?, .invalidServerURL?: false
        default: true
        }
    }

    private func existing(_ file: UploadFile, since: Date, excluding claimed: Set<Document.ID>) async throws
        -> Document?
    {
        let filter = DocumentFilter(createdAfter: since.addingTimeInterval(-Self.clockSkew))
        return try await service.documents(filter: filter)
            .filter { $0.originalFilename == file.filename && !claimed.contains($0.id) }
            .min { $0.createdAt < $1.createdAt }
    }

    /// A document created by an earlier attempt; if it can't be read back, what the upload sent describes it.
    private func resumed(_ id: Document.ID, file: UploadFile) async -> Document {
        if let document = try? await service.document(id: id) {
            return document
        }
        return Document(
            id: id, title: file.title ?? file.filename, originalFilename: file.filename, mimeType: file.mimeType)
    }
}
