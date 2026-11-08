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

    enum CodingKeys: String, CodingKey {
        case id, name, role
        case createdAt = "created_at"
    }

    public init(id: UUID, name: String, role: Role, createdAt: Date? = nil) {
        self.id = id
        self.name = name
        self.role = role
        self.createdAt = createdAt
    }
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
