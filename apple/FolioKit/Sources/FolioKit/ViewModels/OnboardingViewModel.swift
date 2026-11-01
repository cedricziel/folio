import Foundation
import Observation

@MainActor @Observable
public final class OnboardingViewModel {
    public enum State: Equatable {
        case idle
        case checking
        case failed(String)
    }

    public var address = ""
    public private(set) var state = State.idle

    private let store: any ServerConfigStore
    private let makeService: (URL) -> any FolioService
    private let onConnected: (URL) -> Void

    public init(
        store: any ServerConfigStore,
        makeService: @escaping (URL) -> any FolioService,
        onConnected: @escaping (URL) -> Void
    ) {
        self.store = store
        self.makeService = makeService
        self.onConnected = onConnected
        address = store.load()?.absoluteString ?? ""
    }

    public func connect() async {
        state = .checking
        do {
            let url = try ServerAddress.parse(address)
            try await makeService(url).ping()
            store.save(url)
            state = .idle
            onConnected(url)
        } catch {
            state = .failed(error.localizedDescription)
        }
    }
}
