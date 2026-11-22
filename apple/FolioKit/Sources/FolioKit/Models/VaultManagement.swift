import Foundation

public struct VaultMetadata: Identifiable, Codable, Equatable, Sendable {
    public let id: UUID
    public let name: String
    public let createdBy: String?
    public let createdAt: Date
    public let updatedBy: String?
    public let updatedAt: Date
    public let deletedAt: Date?

    enum CodingKeys: String, CodingKey {
        case id, name
        case createdBy = "created_by"
        case createdAt = "created_at"
        case updatedBy = "updated_by"
        case updatedAt = "updated_at"
        case deletedAt = "deleted_at"
    }
}

public enum VaultPrincipalType: String, Codable, CaseIterable, Sendable {
    case user, group
    public var title: String { rawValue.capitalized }
}

public struct VaultMember: Identifiable, Codable, Equatable, Sendable {
    public var id: String { "\(principalType.rawValue):\(principalID)" }
    public let vaultID: UUID
    public let principalType: VaultPrincipalType
    public let principalID: String
    public let role: Vault.Role
    public let grantedBy: String?
    public let grantedAt: Date

    enum CodingKeys: String, CodingKey {
        case vaultID = "vault_id"
        case principalType = "principal_type"
        case principalID = "principal_id"
        case role
        case grantedBy = "granted_by"
        case grantedAt = "granted_at"
    }
}

public struct VaultHistoryEntry: Identifiable, Decodable, Equatable, Sendable {
    public let id: UUID
    public let at: Date
    public let actor: String?
    public let action: String
    public let details: Details

    public struct Details: Decodable, Equatable, Sendable {
        public let name: String?
        public let principalID: String?
        public let role: String?

        enum CodingKeys: String, CodingKey {
            case name, role
            case principalID = "principal_id"
        }
    }

    public var summary: String {
        let principal = details.principalID ?? "member"
        switch action {
        case "vault.created": return "Created \(details.name ?? "vault")"
        case "vault.updated": return "Renamed to \(details.name ?? "vault")"
        case "vault.deleted": return "Deleted \(details.name ?? "vault")"
        case "vault.restored": return "Restored \(details.name ?? "vault")"
        case "vault_membership.granted": return "Granted \(details.role ?? "access") to \(principal)"
        case "vault_membership.role_changed": return "Changed \(principal) to \(details.role ?? "member")"
        case "vault_membership.revoked": return "Revoked access for \(principal)"
        case "document.moved": return "Moved a document"
        default: return "Vault activity"
        }
    }
}
