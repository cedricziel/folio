import Foundation

public struct SourceActivity: Identifiable, Sendable, Equatable {
    public enum Health: Sendable, Equatable {
        case off, notChecked, available, unavailable
        public var title: String {
            switch self {
            case .off: "Off"
            case .notChecked: "Not checked yet"
            case .available: "Available"
            case .unavailable: "Source unavailable"
            }
        }
    }
    public let id: UUID
    public let vaultID: UUID
    public let name: String
    public let health: Health
    public let lastCheckedAt: Date?

    public init(id: UUID, vaultID: UUID, name: String, health: Health, lastCheckedAt: Date?) {
        self.id = id
        self.vaultID = vaultID
        self.name = name
        self.health = health
        self.lastCheckedAt = lastCheckedAt
    }
}

public protocol SourceActivityService: SourceOutcomeService {
    func sourceActivity(vaultID: UUID) async throws -> [SourceActivity]
}
