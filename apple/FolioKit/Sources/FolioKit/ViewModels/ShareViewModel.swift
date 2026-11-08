import Foundation
import Observation

/// Drives the compact share sheet: title, tag picker and "Upload".
@MainActor @Observable
public final class ShareViewModel {
    public enum State: Equatable {
        case idle
        case uploading
        /// Handed to the background session; the outcome arrives in `retained` later.
        case queued
        /// The server accepted every file.
        case done
        case failed(String)
    }

    /// How far a stopped batch got before it failed.
    public struct BatchProgress: Equatable, Sendable {
        public let uploaded: Int
        public let total: Int
    }

    /// A batch that stopped part-way. Retrying resumes it with its original files, vault and tags.
    private struct Batch {
        let files: [UploadFile]
        let tagIDs: [Tag.ID]
        let vaultID: UUID?
        let service: any FolioService
        var queued = 0
    }

    public var title: String
    public var selectedTagIDs: Set<Tag.ID> = []
    private var loadedTags: [Tag] = []
    public private(set) var state = State.idle
    /// Earlier background shares the server has not accepted, plus the outcome of retries started here.
    public private(set) var retained: [ShareUploadRecord] = []
    public private(set) var retrying: Set<ShareUploadRecord.ID> = []
    public private(set) var retryError: String?
    private var retried: Set<ShareUploadRecord.ID> = []
    /// Set while a batch has stopped part-way; `upload()` then finishes only its remaining steps.
    public private(set) var progress: BatchProgress?
    private var unfinished: Batch?
    private let progressStore: any UploadProgressStoring
    private let progressScope: String
    public let files: [UploadFile]
    public let destination: VaultDestinationViewModel

    private let background: any BackgroundUploading
    private var tagsVaultID: UUID?

    /// Title, vault and tags are fixed once a batch has partly gone through, so a retry cannot duplicate it.
    public var hasUnfinishedBatch: Bool { unfinished != nil }

    public var tags: [Tag] {
        tagsVaultID == destination.selectedVaultID ? loadedTags : []
    }

    public init(
        files: [UploadFile], service: any FolioService, background: any BackgroundUploading,
        destination: VaultDestinationViewModel? = nil,
        progressStore: any UploadProgressStoring = InMemoryUploadProgressStore(), progressScope: String = ""
    ) {
        self.files = files
        self.background = background
        self.progressStore = progressStore
        self.progressScope = progressScope
        self.destination = destination ?? VaultDestinationViewModel(service: service)
        title = files.count == 1 ? files[0].filename : ""
        refreshRetained()
    }

    public func refreshRetained() {
        retained = background.retainedUploads().filter { $0.status != .uploaded || retried.contains($0.id) }
    }

    public func retry(_ id: ShareUploadRecord.ID) async {
        guard !retrying.contains(id) else { return }
        retrying.insert(id)
        retryError = nil
        defer { retrying.remove(id) }
        do {
            try await background.retry(id)
            retried.insert(id)
        } catch {
            retryError = error.localizedDescription
        }
        refreshRetained()
    }

    public func discard(_ id: ShareUploadRecord.ID) {
        background.discard(id)
        retried.remove(id)
        refreshRetained()
    }

    public func loadTags() async {
        await destination.load()
        let vaultID = destination.selectedVaultID
        loadedTags = []
        selectedTagIDs = []
        guard destination.canTag else { return }
        let loaded = (try? await destination.service.tags()) ?? []
        guard vaultID == destination.selectedVaultID, !Task.isCancelled else { return }
        loadedTags = loaded
        tagsVaultID = vaultID
    }

    /// Tags can only be attached once the server has created the document, so tagged uploads run in the
    /// foreground. Untagged ones are handed to the background session and survive the sheet closing.
    ///
    /// After a partial failure, calling this again resumes the same batch: files already handed off or uploaded
    /// are not sent again, and tags already attached are skipped.
    public func upload() async {
        guard state != .uploading else { return }
        state = .uploading
        let batch: Batch
        if let unfinished {
            batch = unfinished
        } else {
            await destination.load()
            guard destination.canUpload else {
                state = .failed(destination.error ?? "Choose a vault you can upload to.")
                return
            }
            let vaultID = destination.selectedVaultID
            let tagIDs = destination.canTag && tagsVaultID == vaultID ? selectedTagIDs : []
            batch = Batch(
                files: titled(), tagIDs: tagIDs.sorted { $0.uuidString < $1.uuidString }, vaultID: vaultID,
                service: destination.service
            )
        }
        if batch.tagIDs.isEmpty {
            await queue(batch)
        } else {
            await uploadInForeground(batch)
        }
    }

    private func queue(_ batch: Batch) async {
        var batch = batch
        do {
            for file in batch.files.dropFirst(batch.queued) {
                try await background.enqueue(file, vaultID: batch.vaultID)
                batch.queued += 1
            }
            finish(.queued)
        } catch {
            stop(batch, uploaded: batch.queued, error)
        }
    }

    private func uploadInForeground(_ batch: Batch) async {
        let runner = UploadBatchRunner(service: batch.service, store: progressStore)
        let scope = "\(progressScope)|\(batch.vaultID?.uuidString ?? "default")"
        do {
            _ = try await runner.run(batch.files, tagIDs: batch.tagIDs, scope: scope)
            finish(.done)
        } catch {
            let saved = runner.progress(files: batch.files, tagIDs: batch.tagIDs, scope: scope)
            stop(batch, uploaded: saved?.uploaded ?? 0, error)
        }
    }

    private func finish(_ outcome: State) {
        unfinished = nil
        progress = nil
        state = outcome
    }

    private func stop(_ batch: Batch, uploaded: Int, _ error: any Error) {
        unfinished = batch
        progress = BatchProgress(uploaded: uploaded, total: batch.files.count)
        state = .failed(error.localizedDescription)
    }

    private func titled() -> [UploadFile] {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard files.count == 1, !trimmed.isEmpty else { return files }
        var file = files[0]
        file.title = trimmed
        return [file]
    }
}
