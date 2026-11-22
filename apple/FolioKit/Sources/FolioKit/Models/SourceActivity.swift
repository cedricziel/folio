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
    public enum Origin: String, Sendable, Equatable {
        case config, managed, unknown
    }

    public let id: UUID
    public let vaultID: UUID
    public let name: String
    public let health: Health
    public let lastCheckedAt: Date?
    public let origin: Origin
    public let enabled: Bool
    public let createdAt: Date?
    public let updatedAt: Date?
    public let deletedAt: Date?
    public let createdBy: String?
    public let updatedBy: String?

    public var supportsLifecycle: Bool { origin == .managed && createdAt != nil && updatedAt != nil }

    public init(
        id: UUID, vaultID: UUID, name: String, health: Health, lastCheckedAt: Date?,
        origin: Origin = .unknown, enabled: Bool? = nil,
        createdAt: Date? = nil, updatedAt: Date? = nil, deletedAt: Date? = nil,
        createdBy: String? = nil, updatedBy: String? = nil
    ) {
        self.id = id
        self.vaultID = vaultID
        self.name = name
        self.health = health
        self.lastCheckedAt = lastCheckedAt
        self.origin = origin
        self.enabled = enabled ?? (health != .off)
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.deletedAt = deletedAt
        self.createdBy = createdBy
        self.updatedBy = updatedBy
    }
}

public protocol SourceActivityService: SourceOutcomeService {
    func sourceActivity(vaultID: UUID) async throws -> [SourceActivity]
}
