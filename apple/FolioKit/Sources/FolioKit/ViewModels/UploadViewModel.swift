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
    private var stopRequested = false

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
        stopRequested = false
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
                    _ = try await service.upload(files[index], idempotencyKey: keys[index])
                    completed += 1
                    work?.update(completed: completed)
                }
                state = .finished(count: completed)
                work?.finish(success: true)
            } catch {
                let message = stopRequested ? "Upload stopped." : error.localizedDescription
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

    /// Stops the running batch after the current request; a retry resumes with the same keys.
    public func stop() {
        guard let running else { return }
        stopRequested = true
        running.cancel()
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
