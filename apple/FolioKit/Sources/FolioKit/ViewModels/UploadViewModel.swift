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
    private var batchService: (any FolioService)?
    private var completed = 0

    public var hasStarted: Bool { batchService != nil }

    private let service: any FolioService
    private let onUploaded: () -> Void
    public let destination: VaultDestinationViewModel?

    public init(
        service: any FolioService, destination: VaultDestinationViewModel? = nil,
        onUploaded: @escaping () -> Void
    ) {
        self.service = service
        self.onUploaded = onUploaded
        self.destination = destination
    }

    /// Uploads sequentially; retrying the same failed batch resumes in its original vault.
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
            batchService = destination?.service ?? service
            completed = 0
        }
        guard let service = batchService else { return }
        let previouslyCompleted = completed
        do {
            for file in files.dropFirst(completed) {
                state = .uploading(done: completed, total: files.count)
                _ = try await service.upload(file)
                completed += 1
            }
            state = .finished(count: completed)
        } catch {
            state = .failed(message: error.localizedDescription, uploaded: completed)
        }
        if completed > previouslyCompleted {
            onUploaded()
        }
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
        batchService = nil
        completed = 0
    }
}
