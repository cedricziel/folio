import Foundation
import Observation

@MainActor @Observable
public final class UploadViewModel {
    public enum State: Equatable {
        case idle
        case uploading(done: Int, total: Int)
        case finished(count: Int)
        case failed(message: String, uploaded: Int)
    }

    public private(set) var state = State.idle

    private var batch: [UploadFile] = []
    private var batchKeys: [UUID] = []
    private var batchService: (any FolioService)?
    private var completed = 0
    private var running: Task<Void, Never>?

    public var hasStarted: Bool { batchService != nil }

    private let service: any FolioService
    private let continuedWork: any ContinuedWorkScheduling
    private let onUploaded: () -> Void
    public let destination: VaultDestinationViewModel?

    public init(
        service: any FolioService, destination: VaultDestinationViewModel? = nil,
        continuedWork: any ContinuedWorkScheduling = ForegroundOnlyWork(),
        onUploaded: @escaping () -> Void
    ) {
        self.service = service
        self.continuedWork = continuedWork
        self.onUploaded = onUploaded
        self.destination = destination
    }

    /// Uploads sequentially; retrying the same failed batch resumes in its original vault.
    /// Each file keeps one idempotency key across retries, so an attempt whose answer was lost isn't stored twice.
    public func upload(_ files: [UploadFile]) async {
        guard !files.isEmpty, !isUploading else { return }
        let retrying: Bool
        if case .failed = state {
            retrying = batch == files && batchService != nil
        } else {
            retrying = false
        }
        if !retrying {
            guard destination?.canUpload != false else {
                state = .failed(message: "Choose a vault you can upload to.", uploaded: 0)
                return
            }
            batch = files
            batchKeys = files.map { _ in UUID() }
            batchService = destination?.service ?? service
            completed = 0
        }
        guard let service = batchService else { return }
        let previouslyCompleted = completed
        let keys = batchKeys
        state = .uploading(done: completed, total: files.count)
        let work = continuedWork.begin(title: "Uploading to Folio", total: files.count) { [weak self] in
            self?.stop()
        }
        work?.update(completed: completed)
        let task = Task {
            do {
                for index in completed..<files.count {
                    try Task.checkCancellation()
                    state = .uploading(done: completed, total: files.count)
                    do {
                        _ = try await service.upload(files[index], idempotencyKey: keys[index])
                    } catch FolioError.server(status: 410, message: _) {
                        // The document an earlier attempt stored has been deleted since: upload the file again once.
                        let fresh = UUID()
                        // Kept before the upload: if this answer is lost too, a retry must reuse the same key.
                        batchKeys[index] = fresh
                        _ = try await service.upload(files[index], idempotencyKey: fresh)
                    }
                    completed += 1
                    work?.update(completed: completed)
                }
                state = .finished(count: completed)
                work?.finish(success: true)
            } catch {
                let message = Task.isCancelled ? "Upload stopped." : error.localizedDescription
                state = .failed(message: message, uploaded: completed)
                work?.finish(success: false)
            }
        }
        running = task
        await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            task.cancel()
        }
        running = nil
        if completed > previouslyCompleted {
            onUploaded()
        }
    }

    /// Cancels the running batch, including its request in flight. A retry resumes with the same keys, so a file
    /// the server stored before the cancellation isn't stored twice.
    public func stop() {
        running?.cancel()
    }

    private var isUploading: Bool {
        if case .uploading = state {
            return true
        }
        return false
    }

    public func dismiss() {
        guard !isUploading else { return }
        state = .idle
        batch = []
        batchKeys = []
        batchService = nil
        completed = 0
    }
}
