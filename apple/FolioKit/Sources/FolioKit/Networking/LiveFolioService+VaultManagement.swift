import Foundation
import OpenAPIRuntime

extension LiveFolioService: VaultManagementService {
    public func vaultCatalog() async throws -> VaultCatalog {
        let catalog: VaultCatalog = try await vaultResponse(path: "api/vaults?include_deleted=true")
        guard
            catalog.defaultVaultID.map({ id in catalog.vaults.contains(where: { $0.id == id }) })
                ?? catalog.vaults.isEmpty
        else { throw FolioError.unreadableResponse }
        return catalog
    }

    public func vaultMetadata(id: UUID) async throws -> VaultMetadata {
        let metadata: VaultMetadata = try await vaultResponse(path: vaultPath(id))
        guard metadata.id == id else { throw FolioError.unreadableResponse }
        return metadata
    }

    public func createVault(name: String) async throws -> VaultMetadata {
        try await vaultResponse(path: "api/vaults", method: "POST", body: JSONEncoder().encode(NameInput(name: name)))
    }

    public func renameVault(id: UUID, name: String) async throws -> VaultMetadata {
        let metadata: VaultMetadata = try await vaultResponse(
            path: vaultPath(id), method: "PUT", body: JSONEncoder().encode(NameInput(name: name)))
        guard metadata.id == id else { throw FolioError.unreadableResponse }
        return metadata
    }

    public func deleteVault(id: UUID) async throws {
        _ = try await managementRequest(path: vaultPath(id), method: "DELETE")
    }

    public func restoreVault(id: UUID) async throws -> VaultMetadata {
        let metadata: VaultMetadata = try await vaultResponse(path: "\(vaultPath(id))/restore", method: "POST")
        guard metadata.id == id else { throw FolioError.unreadableResponse }
        return metadata
    }

    public func setDefaultVault(id: UUID) async throws {
        _ = try await managementRequest(path: "\(vaultPath(id))/default", method: "POST")
    }

    public func vaultMembers(id: UUID) async throws -> [VaultMember] {
        let members: [VaultMember] = try await vaultResponse(path: "\(vaultPath(id))/members")
        guard members.allSatisfy({ $0.vaultID == id }) else { throw FolioError.unreadableResponse }
        return members
    }

    public func grantVaultMember(
        vaultID: UUID, principalType: VaultPrincipalType, principalID: String, role: Vault.Role
    ) async throws {
        _ = try await managementRequest(
            path: "\(vaultPath(vaultID))/members", method: "POST",
            body: JSONEncoder().encode(MemberInput(principalType: principalType, principalID: principalID, role: role))
        )
    }

    public func revokeVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String) async throws {
        let encoded = principalID.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? ""
        guard !encoded.isEmpty else { throw FolioError.unreadableResponse }
        _ = try await managementRequest(
            path: "\(vaultPath(vaultID))/members/\(principalType.rawValue)/\(encoded)", method: "DELETE")
    }

    public func vaultHistory(id: UUID) async throws -> [VaultHistoryEntry] {
        try await vaultResponse(path: "\(vaultPath(id))/history")
    }

    private func vaultPath(_ id: UUID) -> String { "api/vaults/\(id.uuidString.lowercased())" }

    private func vaultResponse<T: Decodable>(path: String, method: String = "GET", body: Data? = nil) async throws -> T
    {
        let data = try await managementRequest(path: path, method: method, body: body)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            try LenientDateTranscoder().decode(decoder.singleValueContainer().decode(String.self))
        }
        do { return try decoder.decode(T.self, from: data) } catch { throw FolioError.unreadableResponse }
    }
}

private struct NameInput: Encodable { let name: String }
private struct MemberInput: Encodable {
    let principalType: VaultPrincipalType
    let principalID: String
    let role: Vault.Role
    enum CodingKeys: String, CodingKey {
        case principalType = "principal_type"
        case principalID = "principal_id"
        case role
    }
}
