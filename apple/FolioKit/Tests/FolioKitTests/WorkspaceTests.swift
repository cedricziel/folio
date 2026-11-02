import Foundation
import Testing

@testable import FolioKit

@MainActor struct WorkspaceTests {
    private let personal = Vault(id: UUID(), name: "Personal", role: .owner)
    private let shared = Vault(id: UUID(), name: "Household", role: .viewer)

    @Test func scopedExternalLinkOpensItsVaultAndDocument() async {
        let document = Document(title: "Shared letter")
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id),
            vaultServices: [personal.id: MockFolioService(), shared.id: MockFolioService(documents: [document])]
        )
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()

        workspace.open(document.id, vaultID: shared.id)
        await workspace.session.list.load()

        #expect(workspace.selectedVaultID == shared.id)
        #expect(workspace.destination == .all)
        #expect(workspace.session.path == [document.id])
        #expect(workspace.session.list.rows.map(\.id) == [document.id])
    }

    @Test func inaccessibleExternalVaultDoesNotFallBackToTheVisibleLibrary() async {
        let source = MockFolioService(vaults: VaultCatalog(vaults: [personal], defaultVaultID: personal.id))
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()

        workspace.open(UUID(), vaultID: UUID())

        #expect(workspace.selectedVaultID == personal.id)
        #expect(workspace.destination == .inbox)
        #expect(workspace.session.path.isEmpty)
        #expect(workspace.navigationError == "This vault is no longer available.")
    }

    @Test func documentIntentOpensInTheServerDefaultVault() async {
        let document = Document(title: "Default-vault letter")
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id),
            vaultServices: [personal.id: MockFolioService(documents: [document]), shared.id: MockFolioService()]
        )
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()
        workspace.selectVault(shared.id)

        workspace.open(document.id)
        await workspace.session.list.load()

        #expect(workspace.selectedVaultID == personal.id)
        #expect(workspace.destination == .all)
        #expect(workspace.session.path == [document.id])
        #expect(workspace.session.list.rows.map(\.id) == [document.id])
    }

    @Test func discoverySelectsServerDefaultAndScopesTheLibrary() async {
        let document = Document(title: "Shared letter")
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: shared.id),
            vaultServices: [personal.id: MockFolioService(), shared.id: MockFolioService(documents: [document])]
        )
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()
        await workspace.session.list.load()

        #expect(workspace.selectedVaultID == shared.id)
        #expect(workspace.session.list.rows.map(\.id) == [document.id])
        #expect(!workspace.canEdit)
        #expect(workspace.canUpload)
    }

    @Test func switchingVaultsClearsSearchSelectionAndBothNavigationPaths() async {
        let source = MockFolioService(vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id))
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()
        workspace.session.list.searchText = "private search"
        workspace.session.selection = UUID()
        workspace.session.path = [UUID()]
        workspace.destination = .all
        workspace.session.list.searchText = "private all search"
        workspace.session.path = [UUID()]

        workspace.selectVault(shared.id)

        #expect(workspace.destination == .inbox)
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.selection == nil)
        #expect(workspace.session.path.isEmpty)
        workspace.destination = .all
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.path.isEmpty)
    }

    @Test func lateResultsFromPreviousVaultCannotReplaceSelectedVaultDocuments() async {
        let personalService = MockFolioService(documents: [Document(title: "Private letter")])
        let sharedDocument = Document(title: "Shared letter")
        let sharedService = MockFolioService(documents: [sharedDocument])
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id),
            vaultServices: [personal.id: personalService, shared.id: sharedService]
        )
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()
        await personalService.suspendDocuments()
        let oldList = workspace.session.list
        let oldLoad = Task { await oldList.load() }
        await personalService.waitForDocumentRequest()

        workspace.selectVault(shared.id)
        await workspace.session.list.load()
        await personalService.releaseDocuments()
        await oldLoad.value

        #expect(workspace.session.list.rows.map(\.id) == [sharedDocument.id])
    }

    @Test func uploadDestinationDoesNotChangeTheLibraryVault() async {
        let source = MockFolioService(vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: shared.id))
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()
        let destination = workspace.uploadDestination()

        destination.selectVault(personal.id)

        #expect(destination.selectedVaultID == personal.id)
        #expect(destination.canUpload)
        #expect(workspace.selectedVaultID == shared.id)
        #expect(!workspace.canEdit)
    }

    @Test func discoveryFailureDoesNotExposeDefaultDocuments() async {
        let source = MockFolioService()
        await source.setFailure(.unreachable)
        let workspace = WorkspaceViewModel(service: source)

        await workspace.loadVaults()

        #expect(!workspace.isReady)
        #expect(workspace.discoveryError == "Couldn't reach the server.")
    }

    @Test func emptyCatalogDoesNotGrantUploadOrEditing() async {
        let service = MockFolioService(vaults: .empty)
        let workspace = WorkspaceViewModel(service: service)
        await workspace.loadVaults()
        let destination = workspace.uploadDestination()

        #expect(!workspace.canEdit)
        #expect(!workspace.canUpload)
        #expect(!destination.canUpload)
        #expect(!destination.canTag)
    }

    @Test func firstSessionOpensInbox() {
        let workspace = WorkspaceViewModel(service: MockFolioService())
        #expect(workspace.destination == .inbox)
        #expect(workspace.session.list.scope == .inbox)
        #expect(!workspace.canEdit)
        #expect(!workspace.canUpload)
    }

    @Test func switchingDestinationsPreservesEachBrowser() {
        let workspace = WorkspaceViewModel(service: MockFolioService())
        let documentID = UUID()
        workspace.session.list.searchText = "electricity"
        workspace.session.selection = documentID
        workspace.session.path = [documentID]

        workspace.destination = .all
        #expect(workspace.session.list.scope == .all)
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.path.isEmpty)
        workspace.session.list.searchText = "insurance"

        workspace.destination = .inbox
        #expect(workspace.session.list.searchText == "electricity")
        #expect(workspace.session.selection == documentID)
        #expect(workspace.session.path == [documentID])
        workspace.session.path = []
        #expect(workspace.session.selection == documentID)

        workspace.destination = .all
        #expect(workspace.session.list.searchText == "insurance")
    }

    @Test func openingDocumentUsesUnfilteredLibraryWithoutErasingInbox() async {
        let workspace = WorkspaceViewModel(service: MockFolioService())
        await workspace.loadVaults()
        workspace.session.list.searchText = "inbox search"
        let documentID = UUID()
        workspace.open(documentID)
        #expect(workspace.destination == .all)
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.path == [documentID])
        workspace.destination = .inbox
        #expect(workspace.session.list.searchText == "inbox search")
    }
}
