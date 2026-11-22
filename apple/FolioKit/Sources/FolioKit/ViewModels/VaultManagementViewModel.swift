import Foundation
import Observation

@MainActor @Observable
public final class VaultManagementViewModel {
    public private(set) var catalog = VaultCatalog.empty
    public private(set) var catalogRevision = 0
    public private(set) var metadata: VaultMetadata?
    public private(set) var members: [VaultMember] = []
    public private(set) var history: [VaultHistoryEntry] = []
    public private(set) var isLoading = false
    public private(set) var isLoadingDetails = false
    public private(set) var isMutating = false
    public private(set) var hasLoaded = false
    public private(set) var error: String?
    public private(set) var detailError: String?
    private let service: any VaultManagementService
    private var catalogRequest = UUID()
    private var detailRequest = UUID()

    public init(service: any VaultManagementService) { self.service = service }

    public func canManage(_ id: UUID) -> Bool {
        catalog.vaults.contains { $0.id == id && $0.role == .owner }
    }

    public func load() async {
        let request = UUID()
        catalogRequest = request
        isLoading = true
        error = nil
        defer { if catalogRequest == request { isLoading = false } }
        do {
            let catalog = try await service.vaultCatalog()
            guard catalogRequest == request, !Task.isCancelled else { return }
            self.catalog = catalog
            catalogRevision += 1
            hasLoaded = true
        } catch is CancellationError {
        } catch {
            guard catalogRequest == request, !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    public func loadDetails(id: UUID) async {
        let request = UUID()
        detailRequest = request
        metadata = nil
        members = []
        history = []
        detailError = nil
        isLoadingDetails = true
        defer { if detailRequest == request { isLoadingDetails = false } }
        do {
            let metadata = try await service.vaultMetadata(id: id)
            guard metadata.id == id else { throw FolioError.unreadableResponse }
            guard detailRequest == request, !Task.isCancelled else { return }
            self.metadata = metadata
            if canManage(id) {
                let members = try await service.vaultMembers(id: id)
                guard members.allSatisfy({ $0.vaultID == id }) else { throw FolioError.unreadableResponse }
                guard detailRequest == request, !Task.isCancelled else { return }
                self.members = members
            }
            if canManage(id) || catalog.deletedVaults.contains(where: { $0.id == id }) {
                let history = try await service.vaultHistory(id: id)
                guard detailRequest == request, !Task.isCancelled else { return }
                self.history = history
            }
        } catch is CancellationError {
        } catch {
            guard detailRequest == request, !Task.isCancelled else { return }
            detailError = error.localizedDescription
        }
    }

    public func create(name: String) async -> Bool {
        await mutate {
            _ = try await self.service.createVault(name: name.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    public func rename(id: UUID, name: String) async -> Bool {
        guard canManage(id) else { return false }
        return await mutate(refreshing: id) {
            _ = try await self.service.renameVault(id: id, name: name.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    public func delete(id: UUID) async -> Bool {
        guard canManage(id) else { return false }
        return await mutate(refreshing: id) { try await self.service.deleteVault(id: id) }
    }

    public func restore(id: UUID) async -> Bool {
        guard catalog.deletedVaults.contains(where: { $0.id == id }) else { return false }
        return await mutate(refreshing: id) { _ = try await self.service.restoreVault(id: id) }
    }

    public func makeDefault(id: UUID) async -> Bool {
        guard catalog.vaults.contains(where: { $0.id == id }) else { return false }
        return await mutate { try await self.service.setDefaultVault(id: id) }
    }

    public func grant(id: UUID, type: VaultPrincipalType, principal: String, role: Vault.Role) async -> Bool {
        guard canManage(id) else { return false }
        return await mutate(refreshing: id) {
            try await self.service.grantVaultMember(
                vaultID: id, principalType: type,
                principalID: principal.trimmingCharacters(in: .whitespacesAndNewlines), role: role)
        }
    }

    public func revoke(_ member: VaultMember) async -> Bool {
        guard canManage(member.vaultID) else { return false }
        return await mutate(refreshing: member.vaultID) {
            try await self.service.revokeVaultMember(
                vaultID: member.vaultID, principalType: member.principalType, principalID: member.principalID)
        }
    }

    public func dismissError() { error = nil }

    private func mutate(refreshing id: UUID? = nil, _ action: () async throws -> Void) async -> Bool {
        guard !isMutating else { return false }
        isMutating = true
        error = nil
        defer { isMutating = false }
        do {
            try await action()
            let revision = catalogRevision
            await load()
            if catalogRevision == revision { catalogRevision += 1 }
            if let id { await loadDetails(id: id) }
            return true
        } catch is CancellationError {
            return false
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }
}
