import Foundation
import Observation

/// Upload destination state is independent of workspace navigation.
@MainActor @Observable
public final class VaultDestinationViewModel {
    public private(set) var catalog: VaultCatalog
    public private(set) var selectedVaultID: UUID?
    public private(set) var isReady: Bool
    public private(set) var error: String?
    private let rootService: any FolioService

    public init(service: any FolioService, catalog: VaultCatalog? = nil, selectedVaultID: UUID? = nil) {
        rootService = service
        self.catalog = catalog ?? .empty
        self.selectedVaultID = selectedVaultID ?? catalog?.defaultVaultID
        isReady = catalog != nil
    }

    public var selectedVault: Vault? {
        catalog.vaults.first { $0.id == selectedVaultID }
    }

    public var title: String {
        selectedVault?.name ?? "Library"
    }

    public var canUpload: Bool {
        isReady && selectedVault?.role.canUpload == true
    }

    public var canTag: Bool {
        isReady && selectedVault?.role.canEdit == true
    }

    public var service: any FolioService {
        selectedVaultID.map { rootService.scoped(to: $0) } ?? rootService
    }

    public func selectVault(_ id: UUID) {
        guard catalog.vaults.contains(where: { $0.id == id }) else { return }
        selectedVaultID = id
    }

    public func load() async {
        guard !isReady else { return }
        error = nil
        do {
            let catalog = try await rootService.vaults()
            guard !Task.isCancelled, !isReady else { return }
            self.catalog = catalog
            selectedVaultID = catalog.defaultVaultID
            isReady = true
        } catch {
            guard !Task.isCancelled, !isReady else { return }
            self.error = error.localizedDescription
        }
    }
}
