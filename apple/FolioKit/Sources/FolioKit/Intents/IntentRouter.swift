import Foundation
import Observation

/// Intents that open the app leave a request here; the app's window picks it up and navigates.
@MainActor @Observable
public final class IntentRouter {
    public enum Request: Equatable, Sendable {
        case scan
        case upload
        case inbox
        case document(Document.ID)
    }

    public static let shared = IntentRouter()

    public private(set) var request: Request?

    public init() {}

    public func route(_ request: Request) {
        self.request = request
    }

    public func clear() {
        request = nil
    }
}
