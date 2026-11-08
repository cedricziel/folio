import CryptoKit
import Foundation

/// Where one file of a batch stands: the document the server created for it and the tags still to attach.
public struct UploadStep: Codable, Equatable, Sendable {
    public var documentID: Document.ID?
    public var remainingTags: [Tag.ID]
    /// Sent with every attempt to upload this file. If an answer is lost, the next attempt with the same key gets
    /// the document the server already created instead of a second one.
    public var idempotencyKey: UUID

    public init(documentID: Document.ID? = nil, remainingTags: [Tag.ID], idempotencyKey: UUID = UUID()) {
        self.documentID = documentID
        self.remainingTags = remainingTags
        self.idempotencyKey = idempotencyKey
    }
}

/// Progress of uploading a batch and tagging each new document, kept so a retry resumes where it stopped.
public struct UploadBatchProgress: Codable, Equatable, Sendable {
    public var steps: [UploadStep]
    public var updatedAt: Date

    public var uploaded: Int { steps.filter { $0.documentID != nil }.count }
    public var remainingTags: Int { steps.reduce(0) { $0 + $1.remainingTags.count } }
}

public enum UploadBatchError: Error, Equatable, LocalizedError {
    /// The same files are being uploaded by another Shortcut run or share sheet right now.
    case alreadyRunning

    public var errorDescription: String? {
        "These files are already being uploaded. Try again in a moment."
    }
}

public protocol UploadProgressStoring: Sendable {
    func load(_ key: String) -> UploadBatchProgress?
    func save(_ progress: UploadBatchProgress, for key: String)
    func remove(_ key: String)
    /// Reserves `key` for one run across processes. Returns false while another run holds it.
    func lock(_ key: String) -> Bool
    func unlock(_ key: String)
}

public final class InMemoryUploadProgressStore: UploadProgressStoring, @unchecked Sendable {
    private let mutex = NSLock()
    private var entries: [String: UploadBatchProgress] = [:]
    private var running: Set<String> = []

    public init() {}

    public func load(_ key: String) -> UploadBatchProgress? { mutex.withLock { entries[key] } }
    public func save(_ progress: UploadBatchProgress, for key: String) { mutex.withLock { entries[key] = progress } }
    public func remove(_ key: String) { mutex.withLock { entries[key] = nil } }
    public func lock(_ key: String) -> Bool { mutex.withLock { running.insert(key).inserted } }
    public func unlock(_ key: String) { _ = mutex.withLock { running.remove(key) } }
}

/// Keeps progress in the App Group container, so running a Shortcut again or sharing the same files again resumes
/// the earlier attempt instead of creating its documents twice.
public final class FileUploadProgressStore: UploadProgressStoring, @unchecked Sendable {
    public static let shared = FileUploadProgressStore(
        directory: SharedConfiguration.container ?? FileManager.default.temporaryDirectory)

    /// A run lock left behind by a process that died stops blocking its batch after this.
    static let lockTimeout: TimeInterval = 5 * 60

    private let directory: URL
    private let maxAge: TimeInterval
    private let mutex = NSLock()

    /// - Parameter maxAge: Progress older than this is forgotten; the same files shared later are a new batch.
    public init(directory: URL, maxAge: TimeInterval = 24 * 60 * 60) {
        self.directory = directory.appending(path: "upload-progress", directoryHint: .isDirectory)
        self.maxAge = maxAge
    }

    public func load(_ key: String) -> UploadBatchProgress? {
        mutex.withLock {
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
        mutex.withLock {
            try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try? JSONEncoder().encode(progress).write(to: file(for: key), options: .atomic)
            sweep(olderThan: .now.addingTimeInterval(-maxAge))
        }
    }

    public func remove(_ key: String) {
        mutex.withLock { try? FileManager.default.removeItem(at: file(for: key)) }
    }

    public func lock(_ key: String) -> Bool {
        let file = lockFile(for: key)
        if let modified = Self.modified(file), modified < .now.addingTimeInterval(-Self.lockTimeout) {
            try? FileManager.default.removeItem(at: file)
        }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try Data().write(to: file, options: .withoutOverwriting)
            return true
        } catch {
            return false
        }
    }

    public func unlock(_ key: String) {
        try? FileManager.default.removeItem(at: lockFile(for: key))
    }

    /// Deletes progress nobody resumed in time, so abandoned batches don't pile up in the shared container.
    private func sweep(olderThan cutoff: Date) {
        let files =
            (try? FileManager.default.contentsOfDirectory(
                at: directory, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for file in files where file.pathExtension == "json" {
            if let modified = Self.modified(file), modified < cutoff {
                try? FileManager.default.removeItem(at: file)
            }
        }
    }

    private static func modified(_ file: URL) -> Date? {
        try? file.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
    }

    private func file(for key: String) -> URL {
        directory.appending(path: "\(key).json")
    }

    private func lockFile(for key: String) -> URL {
        directory.appending(path: "\(key).lock")
    }
}

/// Uploads files one at a time and attaches tags to each new document, recording every finished step. Running
/// the same batch again skips uploads and tags that already succeeded.
///
/// Every upload carries its step's `Idempotency-Key`, so an attempt whose answer was lost is answered with the
/// document the server already created; a 409 means the first attempt is still storing it, so the key is kept for
/// the next run. Tag assignment is idempotent on the server, so a tag whose answer was lost is simply sent again. A
/// document deleted before its tags were attached (404 while tagging, 410 for its key) is uploaded again under a new
/// key.
public struct UploadBatchRunner: Sendable {
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
        guard store.lock(key) else { throw UploadBatchError.alreadyRunning }
        defer { store.unlock(key) }
        var progress =
            store.load(key).flatMap { $0.steps.count == files.count ? $0 : nil }
            ?? UploadBatchProgress(steps: files.map { _ in UploadStep(remainingTags: tagIDs) }, updatedAt: now())
        var documents: [Document?] = Array(repeating: nil, count: files.count)

        func record() {
            progress.updatedAt = now()
            store.save(progress, for: key)
        }

        /// Forgets a step's document so it is uploaded again under a new key.
        func restart(_ index: Int) {
            progress.steps[index] = UploadStep(remainingTags: tagIDs)
            documents[index] = nil
            record()
        }

        for index in files.indices {
            var restarted = false
            while true {
                if progress.steps[index].documentID == nil {
                    do {
                        let document = try await service.upload(
                            files[index], idempotencyKey: progress.steps[index].idempotencyKey)
                        documents[index] = document
                        progress.steps[index].documentID = document.id
                        record()
                    } catch FolioError.server(status: 410, message: _) where !restarted {
                        // The document this key created has been deleted since: upload the file again.
                        restarted = true
                        restart(index)
                        continue
                    } catch {
                        record()
                        throw error
                    }
                }
                guard let documentID = progress.steps[index].documentID else { break }
                do {
                    while let tagID = progress.steps[index].remainingTags.first {
                        try await service.addTag(tagID, to: documentID)
                        progress.steps[index].remainingTags.removeFirst()
                        record()
                    }
                    break
                } catch FolioError.server(status: 404, message: _) where !restarted {
                    restarted = true
                    restart(index)
                } catch {
                    record()
                    throw error
                }
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

    /// A document created by an earlier attempt; if it can't be read back, what the upload sent describes it.
    private func resumed(_ id: Document.ID, file: UploadFile) async -> Document {
        if let document = try? await service.document(id: id) {
            return document
        }
        return Document(
            id: id, title: file.title ?? file.filename, originalFilename: file.filename, mimeType: file.mimeType)
    }
}
