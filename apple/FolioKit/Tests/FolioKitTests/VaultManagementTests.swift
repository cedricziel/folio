import Foundation
import Testing

@testable import FolioKit

@Suite(.serialized) struct VaultManagementTests {
    @Test func emptyDiscoveryCanRecoverDeletedVaultsWithoutASelectedVault() async throws {
        VaultURLProtocol.handler = { request in
            #expect(request.url?.path == "/api/vaults")
            #expect(request.url?.query == "include_deleted=true")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == nil)
            return (
                200,
                Data(
                    """
                    {"vaults":[],"default_vault_id":null,"deleted_vaults":[\(Self.metadataJSON)]}
                    """.utf8)
            )
        }
        let catalog = try await service().vaultCatalog()
        #expect(catalog.vaults.isEmpty)
        #expect(catalog.defaultVaultID == nil)
        #expect(catalog.deletedVaults.first?.createdBy == "alice")
    }

    @Test func successfulGrantAcceptsNoContentAndSendsStructuredPrincipalAndRole() async throws {
        VaultURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/vaults/\(Self.id)/members")
            let body = try? JSONSerialization.jsonObject(with: request.httpBody ?? Data()) as? [String: String]
            #expect(body == ["principal_type": "group", "principal_id": "household", "role": "editor"])
            return (204, Data())
        }
        try await service().grantVaultMember(
            vaultID: UUID(uuidString: Self.id)!, principalType: .group, principalID: "household", role: .editor
        )
    }

    @Test func revokeEncodesPrincipalAsOnePathComponent() async throws {
        VaultURLProtocol.handler = { request in
            #expect(request.httpMethod == "DELETE")
            #expect(request.url?.absoluteString.hasSuffix("/members/user/issuer%2Fperson%3Fpart%23fragment") == true)
            return (204, Data())
        }
        try await service().revokeVaultMember(
            vaultID: UUID(uuidString: Self.id)!, principalType: .user, principalID: "issuer/person?part#fragment"
        )
    }

    @MainActor @Test func serverRejectedRevokeKeepsMembersVisible() async {
        let fake = VaultManagementFake()
        let model = VaultManagementViewModel(service: fake)
        await model.load()
        await model.loadDetails(id: UUID(uuidString: Self.id)!)
        let member = model.members.first!
        let changed = await model.revoke(member)
        #expect(!changed)
        #expect(model.members == [member])
        #expect(model.error == "A vault must retain an owner.")
    }

    @MainActor @Test func successfulGrantReloadsCurrentMemberRole() async {
        let model = VaultManagementViewModel(service: VaultManagementFake())
        await model.load()
        await model.loadDetails(id: UUID(uuidString: Self.id)!)
        let changed = await model.grant(
            id: UUID(uuidString: Self.id)!, type: .group, principal: "household", role: .viewer)
        #expect(changed)
        #expect(model.error == nil)
        #expect(model.members.first?.role == .viewer)
    }

    @MainActor @Test func failedCatalogRefreshClearsLibraryAccess() async {
        let vault = Vault(id: UUID(), name: "Household", role: .owner)
        let service = MockFolioService(vaults: .init(vaults: [vault], defaultVaultID: vault.id))
        let workspace = WorkspaceViewModel(service: service)
        await workspace.loadVaults()
        workspace.session.selection = UUID()
        await service.setFailure(.unreachable)
        await workspace.refreshVaults()
        #expect(!workspace.isReady)
        #expect(workspace.selectedVaultID == nil)
        #expect(workspace.session.selection == nil)
        #expect(!workspace.canEdit)
        #expect(!workspace.canUpload)
    }

    @MainActor @Test func changedDefaultKeepsAccessibleSelectedVaultAndQuery() async {
        let first = Vault(id: UUID(), name: "First", role: .owner)
        let second = Vault(id: UUID(), name: "Second", role: .viewer)
        let service = MockFolioService(vaults: .init(vaults: [first, second], defaultVaultID: first.id))
        let workspace = WorkspaceViewModel(service: service)
        await workspace.loadVaults()
        workspace.session.list.searchText = "insurance"
        await service.setVaults(.init(vaults: [first, second], defaultVaultID: second.id))
        await workspace.refreshVaults()
        #expect(workspace.selectedVaultID == first.id)
        #expect(workspace.session.list.searchText == "insurance")
    }

    @Test func oldCatalogDefaultsDeletedVaultsToEmpty() throws {
        let catalog = try JSONDecoder().decode(
            VaultCatalog.self, from: Data("{\"vaults\":[],\"default_vault_id\":null}".utf8))
        #expect(catalog.deletedVaults.isEmpty)
    }

    @MainActor @Test func refreshClearsRevokedVaultAndFallsBackToCurrentDefault() async {
        let first = Vault(id: UUID(), name: "First", role: .owner)
        let second = Vault(id: UUID(), name: "Second", role: .viewer)
        let service = MockFolioService(vaults: .init(vaults: [first, second], defaultVaultID: first.id))
        let workspace = WorkspaceViewModel(service: service)
        await workspace.loadVaults()
        let previousSession = workspace.session.id
        workspace.session.selection = UUID()
        await service.setVaults(.init(vaults: [second], defaultVaultID: second.id))
        await workspace.refreshVaults()
        #expect(workspace.selectedVaultID == second.id)
        #expect(workspace.session.id != previousSession)
        #expect(workspace.session.selection == nil)
        #expect(!workspace.canEdit)
    }

    @MainActor @Test func refreshCanRecoverFromNoActiveVault() async {
        let service = MockFolioService(vaults: .empty)
        let workspace = WorkspaceViewModel(service: service)
        await workspace.loadVaults()
        let restored = Vault(id: UUID(), name: "Restored", role: .owner)
        await service.setVaults(.init(vaults: [restored], defaultVaultID: restored.id))
        await workspace.refreshVaults()
        #expect(workspace.selectedVaultID == restored.id)
        #expect(workspace.canEdit)
    }

    private func service() -> LiveFolioService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [VaultURLProtocol.self]
        return LiveFolioService(
            baseURL: URL(string: "https://folio.invalid")!, session: URLSession(configuration: configuration))
    }

    static let id = "00000000-0000-4000-8000-000000000001"
    static let metadataJSON = """
        {"id":"\(id)","name":"Household","created_by":"alice","updated_by":"bob",
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","deleted_at":"2026-01-03T00:00:00Z"}
        """
    static let memberJSON = """
        {"vault_id":"\(id)","principal_type":"group","principal_id":"household","role":"editor",
        "granted_by":"alice","granted_at":"2026-01-01T00:00:00Z"}
        """
}

private final class VaultURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: ((URLRequest) -> (Int, Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        var copy = request
        if copy.httpBody == nil, let stream = copy.httpBodyStream {
            stream.open()
            defer { stream.close() }
            var data = Data()
            var bytes = [UInt8](repeating: 0, count: 1024)
            while true {
                let count = stream.read(&bytes, maxLength: bytes.count)
                if count <= 0 { break }
                data.append(bytes, count: count)
            }
            copy.httpBody = data
        }
        let (status, data) = Self.handler!(copy)
        client?.urlProtocol(
            self,
            didReceive: HTTPURLResponse(
                url: request.url!, statusCode: status, httpVersion: nil,
                headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private actor VaultManagementFake: VaultManagementService {
    private var grantedMember: VaultMember?
    func vaultCatalog() async throws -> VaultCatalog {
        .init(
            vaults: [Vault(id: UUID(uuidString: VaultManagementTests.id)!, name: "Household", role: .owner)],
            defaultVaultID: UUID(uuidString: VaultManagementTests.id))
    }
    func vaultMetadata(id: UUID) async throws -> VaultMetadata { try decode(VaultManagementTests.metadataJSON) }
    func vaultMembers(id: UUID) async throws -> [VaultMember] {
        if let grantedMember { return [grantedMember] }
        return [try decode(VaultManagementTests.memberJSON)]
    }
    func vaultHistory(id: UUID) async throws -> [VaultHistoryEntry] { [] }
    func revokeVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String) async throws {
        throw FolioError.server(status: 409, message: "A vault must retain an owner.")
    }
    func createVault(name: String) async throws -> VaultMetadata { throw FolioError.unreachable }
    func renameVault(id: UUID, name: String) async throws -> VaultMetadata { throw FolioError.unreachable }
    func deleteVault(id: UUID) async throws { throw FolioError.unreachable }
    func restoreVault(id: UUID) async throws -> VaultMetadata { throw FolioError.unreachable }
    func setDefaultVault(id: UUID) async throws { throw FolioError.unreachable }
    func grantVaultMember(vaultID: UUID, principalType: VaultPrincipalType, principalID: String, role: Vault.Role)
        async throws
    {
        let data = try JSONSerialization.data(withJSONObject: [
            "vault_id": vaultID.uuidString, "principal_type": principalType.rawValue,
            "principal_id": principalID, "role": role.title.lowercased(), "granted_at": "2026-01-01T00:00:00Z",
        ])
        grantedMember = try decode(String(decoding: data, as: UTF8.self))
    }
    private func decode<T: Decodable>(_ string: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(T.self, from: Data(string.utf8))
    }
}
