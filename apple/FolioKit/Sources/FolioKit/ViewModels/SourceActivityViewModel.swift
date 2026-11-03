import Foundation
import Observation

@MainActor @Observable
public final class SourceActivityViewModel {
    public private(set) var sources: [SourceActivity] = []
    public private(set) var isLoading = false
    public private(set) var hasLoaded = false
    public private(set) var error: String?
    private let vaultID: UUID
    private let service: any SourceActivityService
    private var requestID = UUID()

    public init(vaultID: UUID, service: any SourceActivityService) {
        self.vaultID = vaultID
        self.service = service
    }

    public func load() async {
        let request = UUID()
        requestID = request
        isLoading = true
        error = nil
        defer { if requestID == request { isLoading = false } }
        do {
            let sources = try await service.sourceActivity(vaultID: vaultID)
            guard requestID == request, !Task.isCancelled else { return }
            guard sources.allSatisfy({ $0.vaultID == vaultID }) else { throw FolioError.unreadableResponse }
            self.sources = sources
            hasLoaded = true
        } catch is CancellationError {
        } catch {
            guard requestID == request, !Task.isCancelled else { return }
            error = "Could not load sources. Try again."
        }
    }
}
