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

    /// Uploads one file at a time and stops at the first failure.
    public func upload(_ files: [UploadFile]) async {
        guard !files.isEmpty, !isUploading else { return }
        guard destination?.canUpload != false else {
            state = .failed(message: "Choose a vault you can upload to.", uploaded: 0)
            return
        }
        let service = destination?.service ?? service
        var done = 0
        do {
            for file in files {
                state = .uploading(done: done, total: files.count)
                _ = try await service.upload(file)
                done += 1
            }
            state = .finished(count: done)
        } catch {
            state = .failed(message: error.localizedDescription, uploaded: done)
        }
        if done > 0 {
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
        state = .idle
    }
}
