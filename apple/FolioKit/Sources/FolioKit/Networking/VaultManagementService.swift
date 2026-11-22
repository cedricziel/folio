import Foundation

public protocol VaultManagementService: Sendable {
    func vaultCatalog() async throws -> VaultCatalog
    func vaultMetadata(id: UUID) async throws -> VaultMetadata
    func createVault(name: String) async throws -> VaultMetadata
    func renameVault(id: UUID, name: String) async throws -> VaultMetadata
    func deleteVault(id: UUID) async throws
    func restoreVault(id: UUID) async throws -> VaultMetadata
    func setDefaultVault(id: UUID) async throws
    func vaultMembers(id: UUID) async throws -> [VaultMember]
    func grantVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String, role: Vault.Role)
        async throws
    func revokeVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String) async throws
    func vaultHistory(id: UUID) async throws -> [VaultHistoryEntry]
}
