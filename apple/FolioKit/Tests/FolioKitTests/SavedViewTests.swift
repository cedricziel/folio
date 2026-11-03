import Foundation
import Testing

@testable import FolioKit

@MainActor struct SavedViewTests {
    private func view(vault: UUID) -> SavedView {
        SavedView(
            id: UUID(), vaultID: vault, ownerID: "alice", name: "Invoices",
            query: .init(text: "paid", filter: .init(tagIDs: [UUID()]), sort: .title),
            presentation: .grid, createdAt: .distantPast, updatedAt: .distantPast,
            createdBy: "alice", updatedBy: "alice"
        )
    }

    @Test func restoringViewReplacesQueryAndClearsSelection() async {
        let vault = Vault(id: UUID(), name: "Personal", role: .owner)
        let workspace = WorkspaceViewModel(
            service: MockFolioService(vaults: .init(vaults: [vault], defaultVaultID: vault.id))
        )
        await workspace.loadVaults()
        workspace.destination = .all
        workspace.session.selection = UUID()
        workspace.session.path = [UUID()]
        workspace.session.list.searchText = "old query"
        let saved = view(vault: vault.id)

        workspace.selectSavedView(saved)

        #expect(workspace.session.list.query == saved.query)
        #expect(workspace.session.selection == nil)
        #expect(workspace.session.path.isEmpty)
        #expect(workspace.session.presentation == .grid)
        #expect(workspace.selectedSavedViewID == saved.id)
        #expect(!workspace.session.list.canLoadMore)
    }

    @Test func viewFromAnotherVaultDoesNotReplaceCurrentQuery() async {
        let vault = Vault(id: UUID(), name: "Personal", role: .owner)
        let workspace = WorkspaceViewModel(
            service: MockFolioService(vaults: .init(vaults: [vault], defaultVaultID: vault.id))
        )
        await workspace.loadVaults()
        workspace.session.list.searchText = "keep this"

        workspace.selectSavedView(view(vault: UUID()))

        #expect(workspace.session.list.searchText == "keep this")
        #expect(workspace.selectedSavedViewID == nil)
        #expect(workspace.navigationError != nil)
    }

    @Test func delayedSavedViewListCannotRepopulateAnotherVault() async {
        let first = Vault(id: UUID(), name: "Personal", role: .owner)
        let second = Vault(id: UUID(), name: "Shared", role: .viewer)
        let source = MockFolioService(vaults: .init(vaults: [first, second], defaultVaultID: first.id))
        await source.setSavedViews([view(vault: first.id)])
        await source.suspendSavedViews()
        let workspace = WorkspaceViewModel(service: source)
        await workspace.loadVaults()
        let loading = Task { await workspace.loadSavedViews() }
        await source.waitForSavedViewsRequest()

        workspace.selectVault(second.id)
        await source.releaseSavedViews()
        await loading.value

        #expect(workspace.savedViews.isEmpty)
        #expect(!workspace.isLoadingSavedViews)
    }

    @Test func switchingVaultClearsSelectedView() async {
        let first = Vault(id: UUID(), name: "Personal", role: .owner)
        let second = Vault(id: UUID(), name: "Shared", role: .viewer)
        let workspace = WorkspaceViewModel(
            service: MockFolioService(vaults: .init(vaults: [first, second], defaultVaultID: first.id))
        )
        await workspace.loadVaults()
        workspace.selectSavedView(view(vault: first.id))

        workspace.selectVault(second.id)

        #expect(workspace.selectedSavedViewID == nil)
        #expect(workspace.session.list.searchText.isEmpty)
        #expect(workspace.session.presentation == .list)
    }
}
