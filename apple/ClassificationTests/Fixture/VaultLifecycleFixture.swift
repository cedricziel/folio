import FolioKit
import Foundation
import SwiftUI

struct VaultLifecycleFixture: View {
    @State private var service: VaultLifecycleFixtureService

    init() {
        let arguments = ProcessInfo.processInfo.arguments
        _service = State(
            initialValue: VaultLifecycleFixtureService(
                readOnly: arguments.contains("--read-only"), empty: arguments.contains("--vault-empty")
            ))
    }

    var body: some View {
        VaultManagementView(service: service)
            .dynamicTypeSize(ProcessInfo.processInfo.arguments.contains("--large-text") ? .accessibility3 : .large)
    }
}

@MainActor @Observable final class VaultLifecycleFixtureService: VaultManagementService {
    private let id = UUID(uuidString: "00000000-0000-4000-8000-000000000001")!
    private let readOnly: Bool
    private var deleted: Bool
    private var name = "Household"
    private var memberRole = Vault.Role.editor
    private var memberPresent = true
    private var events = ["vault.created"]

    init(readOnly: Bool, empty: Bool) {
        self.readOnly = readOnly
        deleted = empty
    }

    func vaultCatalog() async throws -> VaultCatalog {
        .init(
            vaults: deleted ? [] : [Vault(id: id, name: name, role: readOnly ? .viewer : .owner)],
            defaultVaultID: deleted ? nil : id, deletedVaults: deleted && !readOnly ? [metadata()] : [])
    }

    func vaultMetadata(id: UUID) async throws -> VaultMetadata { metadata() }
    func createVault(name: String) async throws -> VaultMetadata {
        self.name = name
        deleted = false
        events.append("vault.created")
        return metadata()
    }
    func renameVault(id: UUID, name: String) async throws -> VaultMetadata {
        try requireOwner()
        self.name = name
        events.append("vault.updated")
        return metadata()
    }
    func deleteVault(id: UUID) async throws {
        try requireOwner()
        deleted = true
        events.append("vault.deleted")
    }
    func restoreVault(id: UUID) async throws -> VaultMetadata {
        try requireOwner()
        deleted = false
        events.append("vault.restored")
        return metadata()
    }
    func setDefaultVault(id: UUID) async throws {}
    func vaultMembers(id: UUID) async throws -> [VaultMember] {
        try requireOwner()
        return memberPresent
            ? [
                decode(
                    """
                    {"vault_id":"\(self.id)","principal_type":"user","principal_id":"fixture-member","role":"\(memberRole.title.lowercased())",
                    "granted_by":"Morgan","granted_at":"2026-01-01T00:00:00Z"}
                    """)
            ] : []
    }
    func grantVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String, role: Vault.Role)
        async throws
    {
        try requireOwner()
        memberPresent = true
        memberRole = role
        events.append("vault_membership.role_changed")
    }
    func revokeVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String) async throws {
        try requireOwner()
        memberPresent = false
        events.append("vault_membership.revoked")
    }
    func vaultHistory(id: UUID) async throws -> [VaultHistoryEntry] {
        try requireOwner()
        return events.enumerated().map { index, action in
            decode(
                """
                {"id":"00000000-0000-4000-8000-\(String(format: "%012d", index + 1))","at":"2026-01-01T00:00:00Z",
                "actor":"Morgan","action":"\(action)","details":{"name":"\(name)","principal_id":"fixture-member","role":"viewer"}}
                """)
        }
    }
    private func metadata() -> VaultMetadata {
        decode(
            """
            {"id":"\(id)","name":"\(name)","created_by":"Morgan","updated_by":"Taylor",
            "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","deleted_at":\(deleted ? "\"2026-01-03T00:00:00Z\"" : "null")}
            """)
    }
    private func requireOwner() throws {
        if readOnly { throw FolioError.server(status: 403, message: "Only a vault owner can manage this vault") }
    }
    private func decode<T: Decodable>(_ json: String) -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try! decoder.decode(T.self, from: Data(json.utf8))
    }
}
