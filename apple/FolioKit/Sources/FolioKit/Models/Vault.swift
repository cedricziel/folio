import Foundation

public struct Vault: Identifiable, Codable, Equatable, Sendable {
    /// A role this app version doesn't know decodes as `.unknown` and gets read-only access.
    public enum Role: Codable, Hashable, Sendable {
        case viewer, contributor, editor, owner
        case unknown(String)

        public init(from decoder: Decoder) throws {
            let raw = try decoder.singleValueContainer().decode(String.self)
            self = Self.known[raw] ?? .unknown(raw)
        }

        public func encode(to encoder: Encoder) throws {
            var container = encoder.singleValueContainer()
            try container.encode(rawValue)
        }

        private static let known: [String: Role] = [
            "viewer": .viewer, "contributor": .contributor, "editor": .editor, "owner": .owner,
        ]

        private var rawValue: String {
            switch self {
            case .viewer: "viewer"
            case .contributor: "contributor"
            case .editor: "editor"
            case .owner: "owner"
            case let .unknown(raw): raw
            }
        }

        public var title: String {
            switch self {
            case .unknown: "Read only"
            default: rawValue.capitalized
            }
        }

        public var canUpload: Bool {
            self == .contributor || canEdit
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
    public let deletedVaults: [VaultMetadata]

    enum CodingKeys: String, CodingKey {
        case vaults
        case defaultVaultID = "default_vault_id"
        case deletedVaults = "deleted_vaults"
    }

    public init(vaults: [Vault], defaultVaultID: UUID?, deletedVaults: [VaultMetadata] = []) {
        self.vaults = vaults
        self.defaultVaultID = defaultVaultID
        self.deletedVaults = deletedVaults
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        vaults = try container.decode([Vault].self, forKey: .vaults)
        defaultVaultID = try container.decodeIfPresent(UUID.self, forKey: .defaultVaultID)
        deletedVaults = try container.decodeIfPresent([VaultMetadata].self, forKey: .deletedVaults) ?? []
    }

    public static let empty = VaultCatalog(vaults: [], defaultVaultID: nil)
}
