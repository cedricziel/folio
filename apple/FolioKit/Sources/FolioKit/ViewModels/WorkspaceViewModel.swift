import Foundation
import Observation

/// Library destinations currently backed by the document service.
public enum WorkspaceDestination: String, CaseIterable, Sendable {
    case inbox
    case all

    public var title: String {
        switch self {
        case .inbox: "Inbox"
        case .all: "All documents"
        }
    }

    public var systemImage: String {
        switch self {
        case .inbox: "tray"
        case .all: "doc.on.doc"
        }
    }
}

/// Browser state retained while another destination is visible.
@MainActor @Observable
public final class DocumentBrowserSession {
    public let list: DocumentListViewModel
    public var selection: Document.ID?
    public var path: [Document.ID] = []

    init(service: any FolioService, destination: WorkspaceDestination) {
        list = DocumentListViewModel(service: service)
        list.scope = destination == .inbox ? .inbox : .all
    }
}

/// In-memory navigation for one signed-in workspace; discard it on account changes.
@MainActor @Observable
public final class WorkspaceViewModel {
    public var destination = WorkspaceDestination.inbox
    public private(set) var catalog = VaultCatalog.empty
    public private(set) var selectedVaultID: UUID?
    public private(set) var isReady = false
    public private(set) var discoveryError: String?
    public private(set) var navigationError: String?
    public private(set) var thumbnails: ThumbnailCache?
    private let rootService: any FolioService
    private var inbox: DocumentBrowserSession
    private var all: DocumentBrowserSession

    public init(service: any FolioService) {
        rootService = service
        inbox = DocumentBrowserSession(service: service, destination: .inbox)
        all = DocumentBrowserSession(service: service, destination: .all)
    }

    public var selectedVault: Vault? {
        catalog.vaults.first { $0.id == selectedVaultID }
    }

    public var service: any FolioService {
        selectedVaultID.map { rootService.scoped(to: $0) } ?? rootService
    }

    public var canEdit: Bool {
        isReady && selectedVault?.role.canEdit == true
    }

    public var canUpload: Bool {
        isReady && catalog.vaults.contains { $0.role.canUpload }
    }

    public func loadVaults() async {
        guard !isReady else { return }
        discoveryError = nil
        do {
            let catalog = try await rootService.vaults()
            guard !Task.isCancelled, !isReady else { return }
            self.catalog = catalog
            selectedVaultID = catalog.defaultVaultID
            resetBrowsers()
            isReady = true
        } catch {
            guard !Task.isCancelled, !isReady else { return }
            discoveryError = error.localizedDescription
        }
    }

    public func selectVault(_ id: UUID) {
        guard id != selectedVaultID, catalog.vaults.contains(where: { $0.id == id }) else { return }
        selectedVaultID = id
        destination = .inbox
        resetBrowsers()
    }

    public func uploadDestination() -> VaultDestinationViewModel {
        VaultDestinationViewModel(service: rootService, catalog: catalog, selectedVaultID: selectedVaultID)
    }

    private func resetBrowsers() {
        thumbnails = selectedVaultID.map { ThumbnailCache(service: service, vaultID: $0) }
        inbox = DocumentBrowserSession(service: service, destination: .inbox)
        all = DocumentBrowserSession(service: service, destination: .all)
    }

    public var session: DocumentBrowserSession {
        destination == .inbox ? inbox : all
    }

    /// Opens a document in the supplied vault or the server default used by document intents.
    public func open(_ id: Document.ID, vaultID: UUID? = nil) {
        if let vaultID = vaultID ?? catalog.defaultVaultID {
            guard catalog.vaults.contains(where: { $0.id == vaultID }) else {
                navigationError = "This vault is no longer available."
                return
            }
            selectVault(vaultID)
        }
        navigationError = nil
        destination = .all
        all.list.searchText = ""
        all.selection = id
        all.path = [id]
    }

    public func dismissNavigationError() {
        navigationError = nil
    }
}
