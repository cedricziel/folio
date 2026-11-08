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
public final class DocumentBrowserSession: Identifiable {
    public let id = UUID()
    public let list: DocumentListViewModel
    public var savedViewName: String?
    public var presentation = SavedView.Presentation.list
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
    public var destination = WorkspaceDestination.inbox {
        didSet {
            if selectedSavedViewID != nil {
                selectedSavedViewID = nil
                all = DocumentBrowserSession(service: service, destination: .all)
            }
        }
    }
    public private(set) var selectedSavedViewID: UUID?
    public private(set) var catalog = VaultCatalog.empty
    public private(set) var selectedVaultID: UUID?
    public private(set) var isReady = false
    public private(set) var discoveryError: String?
    public private(set) var navigationError: String?
    public private(set) var thumbnails: ThumbnailCache?

    public private(set) var savedViews: [SavedView] = []
    public private(set) var savedViewsError: String?
    public private(set) var isLoadingSavedViews = false
    public private(set) var hasMoreSavedViews = false
    public private(set) var isSavingView = false
    private var viewGeneration = 0
    private var viewsAfter: UUID?
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

        selectedSavedViewID = nil
        viewGeneration += 1
        savedViews = []
        savedViewsError = nil
        isLoadingSavedViews = false
        isSavingView = false
        hasMoreSavedViews = false
        viewsAfter = nil
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

    public func loadSavedViews(more: Bool = false) async {
        guard isReady, !isLoadingSavedViews, !more || hasMoreSavedViews else { return }
        let generation = viewGeneration
        let source = service
        isLoadingSavedViews = true
        savedViewsError = nil
        defer { if generation == viewGeneration { isLoadingSavedViews = false } }
        do {
            let page = try await source.savedViews(after: more ? viewsAfter : nil)
            guard generation == viewGeneration, !Task.isCancelled else { return }
            guard page.allSatisfy({ $0.vaultID == selectedVaultID }) else { throw FolioError.unreadableResponse }
            if more {
                let known = Set(savedViews.map(\.id))
                savedViews.append(contentsOf: page.filter { !known.contains($0.id) })
            } else {
                savedViews = page
            }
            hasMoreSavedViews = page.count == 50
            viewsAfter = page.last?.id
        } catch {
            guard generation == viewGeneration, !Task.isCancelled else { return }
            savedViewsError = error.localizedDescription
        }
    }

    public func openSavedView(_ id: UUID) async {
        let generation = viewGeneration
        let source = service
        do {
            let view = try await source.savedView(id: id)
            guard generation == viewGeneration, !Task.isCancelled else { return }
            selectSavedView(view)
        } catch {
            guard generation == viewGeneration, !Task.isCancelled else { return }
            if selectedSavedViewID == id { destination = .all }
            navigationError = error.localizedDescription
        }
    }

    @discardableResult
    public func saveView(_ draft: SavedViewDraft, replacing id: UUID? = nil) async -> Bool {
        guard isReady, !isSavingView else { return false }
        let generation = viewGeneration
        let source = service
        isSavingView = true
        savedViewsError = nil
        defer { if generation == viewGeneration { isSavingView = false } }
        do {
            let view: SavedView
            if let id {
                view = try await source.updateSavedView(id: id, draft: draft)
            } else {
                view = try await source.saveView(draft)
            }
            guard generation == viewGeneration, !Task.isCancelled else { return false }
            guard view.vaultID == selectedVaultID else { throw FolioError.unreadableResponse }
            savedViews.removeAll { $0.id == view.id }
            savedViews.append(view)
            savedViews.sort { $0.id.uuidString < $1.id.uuidString }
            selectSavedView(view)
            return true
        } catch {
            guard generation == viewGeneration, !Task.isCancelled else { return false }
            savedViewsError = error.localizedDescription
            return false
        }
    }

    public func deleteSavedView(_ id: UUID) async {
        let generation = viewGeneration
        let source = service
        do {
            try await source.deleteSavedView(id: id)
            guard generation == viewGeneration, !Task.isCancelled else { return }
            savedViews.removeAll { $0.id == id }
            if selectedSavedViewID == id { destination = .all }
        } catch {
            guard generation == viewGeneration, !Task.isCancelled else { return }
            savedViewsError = error.localizedDescription
        }
    }

    /// Restores a server-validated view in the selected vault, starting at page one.
    public func selectSavedView(_ view: SavedView) {
        guard isReady, view.vaultID == selectedVaultID else {
            navigationError = "This saved view belongs to another vault."
            return
        }
        destination = .all
        all = DocumentBrowserSession(service: service, destination: .all)
        all.list.searchText = view.query.text
        all.list.filter = view.query.filter
        all.list.sort = view.query.sort
        all.presentation = view.presentation
        all.savedViewName = view.name
        all.list.showsSavedView = true
        selectedSavedViewID = view.id
        navigationError = nil
    }

    public func dismissNavigationError() {
        navigationError = nil
    }
}
