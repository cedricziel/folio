import AppIntents
import Foundation

/// A tag, correspondent or document type as App Intents names it: the item and the vault that holds it, since
/// each vault keeps its own. Identifiers saved before vaults existed carry no vault and resolve in the default vault.
public struct VaultScopedID: Hashable, Sendable, EntityIdentifierConvertible {
    public let id: UUID
    public let vaultID: UUID?

    public init(_ id: UUID, vaultID: UUID? = nil) {
        self.id = id
        self.vaultID = vaultID
    }

    /// `<vault>/<id>`, or the bare id without a vault, which is what older Shortcuts saved.
    public var entityIdentifierString: String {
        guard let vaultID else { return id.uuidString.lowercased() }
        return "\(vaultID.uuidString.lowercased())/\(id.uuidString.lowercased())"
    }

    public static func entityIdentifier(for entityIdentifierString: String) -> VaultScopedID? {
        let parts = entityIdentifierString.split(separator: "/", omittingEmptySubsequences: false)
        switch parts.count {
        case 1:
            return UUID(uuidString: entityIdentifierString).map { VaultScopedID($0) }
        case 2:
            guard let vaultID = UUID(uuidString: String(parts[0])), let id = UUID(uuidString: String(parts[1])) else {
                return nil
            }
            return VaultScopedID(id, vaultID: vaultID)
        default:
            return nil
        }
    }
}

/// A value from one vault, with the vault's name when the account has more than one, so pickers can tell
/// same-named tags apart.
public struct VaultScoped<Value: Sendable>: Sendable {
    public let value: Value
    public let vaultID: UUID?
    public let vaultName: String?

    public init(_ value: Value, vaultID: UUID?, vaultName: String? = nil) {
        self.value = value
        self.vaultID = vaultID
        self.vaultName = vaultName
    }
}
