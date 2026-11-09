import Foundation

public struct Vault: Identifiable, Codable, Equatable, Sendable {
    public enum Role: String, Codable, Sendable {
        case viewer, contributor, editor, owner

        public var title: String {
            rawValue.capitalized
        }

        public var canUpload: Bool {
            self != .viewer
        }

        public var canEdit: Bool {
            self == .editor || self == .owner
        }
    }

    public let id: UUID
    public let name: String
    public let role: Role
    /// Absent from older servers.
    public let createdAt: Date?
    /// Whether a grant reaches anyone besides this account. Absent from older servers.
    public let shared: Bool?

    enum CodingKeys: String, CodingKey {
        case id, name, role, shared
        case createdAt = "created_at"
    }

    public init(id: UUID, name: String, role: Role, createdAt: Date? = nil, shared: Bool? = nil) {
        self.id = id
        self.name = name
        self.role = role
        self.createdAt = createdAt
        self.shared = shared
    }

    /// Someone else's vault is shared with this account even when the server doesn't say.
    public var sharing: VaultSharing {
        guard role == .owner else { return .shared }
        switch shared {
        case true?: return .shared
        case false?: return .personal
        case nil: return .unknown
        }
    }
}

/// Whether people besides this account can reach a vault, so the system confirms changes to what it holds.
public enum VaultSharing: Sendable, Equatable {
    case personal, shared, unknown
}

public struct VaultCatalog: Codable, Equatable, Sendable {
    public let vaults: [Vault]
    public let defaultVaultID: UUID?

    enum CodingKeys: String, CodingKey {
        case vaults
        case defaultVaultID = "default_vault_id"
    }

    public init(vaults: [Vault], defaultVaultID: UUID?) {
        self.vaults = vaults
        self.defaultVaultID = defaultVaultID
    }

    public static let empty = VaultCatalog(vaults: [], defaultVaultID: nil)
}
