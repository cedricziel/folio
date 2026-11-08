import Foundation
import Testing

@testable import FolioKit

private typealias Tag = FolioKit.Tag

private let receipts = Tag(name: "Receipts")
private let travel = Tag(name: "Travel")

private func catalog(_ service: MockFolioService) -> DocumentCatalog {
    DocumentCatalog(service: service, progress: InMemoryUploadProgressStore(), uploadScope: "tests")
}

private func ids(_ documents: [Document], vaultID: UUID? = nil) -> [DocumentEntityID] {
    documents.map { DocumentEntityID(documentID: $0.id, vaultID: vaultID) }
}

struct TagChangeTests {
    @Test func addingTouchesOnlyDocumentsWithoutTheTagAndNeverFetchesThem() async throws {
        let untagged = Document(title: "Hotel"), tagged = Document(title: "Taxi", tags: [receipts])
        let service = MockFolioService(documents: [untagged, tagged], tags: [receipts])

        let outcome = try await catalog(service).apply(
            TagChange(.add, tagID: receipts.id, documents: ids([untagged, tagged])))

        #expect(outcome.applied == TagChange(.add, tagID: receipts.id, documents: ids([untagged])))
        #expect(outcome.unchanged == ids([tagged]))
        #expect(await service.tagged == [TagAssignment(tag: receipts.id, document: untagged.id)])
        #expect(await service.documentReads == 0)
    }

    @Test func removingTouchesOnlyDocumentsThatHaveTheTag() async throws {
        let untagged = Document(title: "Hotel"), tagged = Document(title: "Taxi", tags: [receipts])
        let service = MockFolioService(documents: [untagged, tagged], tags: [receipts])

        let outcome = try await catalog(service).apply(
            TagChange(.remove, tagID: receipts.id, documents: ids([untagged, tagged, tagged])))

        #expect(outcome.applied.documents == ids([tagged]))
        #expect(outcome.unchanged == ids([untagged]))
        #expect(await service.untagged == [TagAssignment(tag: receipts.id, document: tagged.id)])
    }

    @Test func eachDocumentIsChangedInItsOwnVaultAndOtherVaultsAreSkipped() async throws {
        let vaultA = UUID(), vaultB = UUID()
        let inA = Document(title: "Lease")
        let scopedA = MockFolioService(documents: [inA], tags: [receipts])
        let scopedB = MockFolioService(documents: [Document(title: "Elsewhere")])
        let root = MockFolioService(vaultServices: [vaultA: scopedA, vaultB: scopedB])
        let elsewhere = DocumentEntityID(documentID: UUID(), vaultID: vaultB)

        let change = TagChange(
            .add, tagID: receipts.id, tagVaultID: vaultA, documents: ids([inA], vaultID: vaultA) + [elsewhere])
        let outcome = try await catalog(root).apply(change)

        #expect(outcome.applied.documents == ids([inA], vaultID: vaultA))
        #expect(outcome.otherVault == [elsewhere])
        #expect(await scopedA.tagged == [TagAssignment(tag: receipts.id, document: inA.id)])
        #expect(await scopedB.tagged.isEmpty)
        #expect(await root.tagged.isEmpty)
    }

    @Test func aFailedDocumentDoesNotStopTheOthers() async throws {
        let kept = Document(title: "Hotel"), gone = Document(title: "Deleted")
        let service = MockFolioService(documents: [kept, gone], tags: [receipts])
        await service.setDeletedDocuments([gone.id])

        let outcome = try await catalog(service).apply(
            TagChange(.add, tagID: receipts.id, documents: ids([gone, kept])))

        #expect(outcome.applied.documents == ids([kept]))
        #expect(outcome.failed == ids([gone]))
    }

    @Test func failingEveryDocumentThrows() async {
        let gone = Document(title: "Deleted")
        let service = MockFolioService(documents: [gone], tags: [receipts])
        await service.setDeletedDocuments([gone.id])

        await #expect(throws: FolioError.self) {
            try await catalog(service).apply(TagChange(.add, tagID: receipts.id, documents: ids([gone])))
        }
    }

    @Test func revertedSwapsAddingAndRemovingForTheSameDocuments() {
        let documents = [DocumentEntityID(documentID: UUID(), vaultID: UUID())]
        let change = TagChange(.add, tagID: receipts.id, tagVaultID: UUID(), documents: documents)
        #expect(change.reverted.kind == .remove)
        #expect(change.reverted.documents == documents)
        #expect(change.reverted.tagVaultID == change.tagVaultID)
        #expect(change.reverted.reverted == change)
    }

    @Test func summaryCountsWhatHappened() {
        let one = [DocumentEntityID(documentID: UUID())]
        let two = one + [DocumentEntityID(documentID: UUID())]
        var outcome = TagChangeOutcome(applied: TagChange(.add, tagID: receipts.id, documents: two))
        #expect(outcome.summary(tagName: "Receipts") == "Tagged 2 documents with Receipts.")

        outcome = TagChangeOutcome(
            applied: TagChange(.remove, tagID: receipts.id, documents: one), unchanged: one, otherVault: two,
            failed: one)
        #expect(
            outcome.summary(tagName: "Receipts")
                == "Removed Receipts from 1 document. 1 didn't have it. 2 are in another vault. 1 couldn't be changed.")
    }
}

struct VaultScopeTests {
    @Test func scopedIdentifiersRoundTripAndAcceptOlderBareIDs() {
        let id = UUID(), vault = UUID()
        let scoped = VaultScopedID(id, vaultID: vault)
        #expect(VaultScopedID.entityIdentifier(for: scoped.entityIdentifierString) == scoped)
        #expect(VaultScopedID.entityIdentifier(for: id.uuidString) == VaultScopedID(id))
        #expect(VaultScopedID(id).entityIdentifierString == id.uuidString.lowercased())
        #expect(VaultScopedID.entityIdentifier(for: "not-a-uuid") == nil)
        #expect(VaultScopedID.entityIdentifier(for: "\(vault.uuidString)/\(id.uuidString)/extra") == nil)
    }

    @Test func intentVaultFollowsWhatItWasGiven() throws {
        let vault = UUID()
        #expect(try DocumentCatalog.vault(asked: nil, implied: [nil, nil]) == nil)
        #expect(try DocumentCatalog.vault(asked: nil, implied: [vault, nil]) == vault)
        #expect(try DocumentCatalog.vault(asked: vault, implied: [vault]) == vault)
        #expect(throws: ShortcutFailure.mixedVaults) {
            try DocumentCatalog.vault(asked: vault, implied: [UUID()])
        }
        #expect(
            ShortcutError(mapping: ShortcutFailure.mixedVaults).message
                == "Those items are in different vaults. Pick them from one vault.")
    }

    @Test func sharingFollowsRoleAndTheServersAnswer() throws {
        #expect(Vault(id: UUID(), name: "Mine", role: .owner, shared: false).sharing == .personal)
        #expect(Vault(id: UUID(), name: "Family", role: .owner, shared: true).sharing == .shared)
        #expect(Vault(id: UUID(), name: "Older server", role: .owner).sharing == .unknown)
        #expect(Vault(id: UUID(), name: "Theirs", role: .viewer).sharing == .shared)

        let json = Data(
            #"""
            {"vaults": [{"id": "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff", "name": "Family", "role": "editor",
                         "shared": true}],
             "default_vault_id": "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff"}
            """#.utf8)
        let decoded = try JSONDecoder().decode(VaultCatalog.self, from: json)
        #expect(decoded.vaults.first?.shared == true)
    }

    @Test func summariesCarryTheirVaultsSharing() async throws {
        let shared = Vault(id: UUID(), name: "Family", role: .owner, shared: true)
        let service = MockFolioService(
            documents: [Document(title: "Lease")], vaults: VaultCatalog(vaults: [shared], defaultVaultID: shared.id))

        let summary = try #require(try await catalog(service).search("lease").first)

        #expect(summary.vaultSharing == .shared)
        #expect(DocumentEntity(summary).sharing == .shared)
    }

    @Test func taxonomyFromEveryVaultIsLabelledAndLookedUpWhereItLives() async throws {
        let home = Vault(id: UUID(), name: "Home", role: .owner)
        let work = Vault(id: UUID(), name: "Work", role: .editor)
        let homeTags = MockFolioService(tags: [receipts])
        let workTags = MockFolioService(tags: [travel])
        let root = MockFolioService(
            vaults: VaultCatalog(vaults: [home, work], defaultVaultID: home.id),
            vaultServices: [home.id: homeTags, work.id: workTags])

        let found = try await catalog(root).acrossVaults { try await $0.tags() }
        #expect(found.map(\.value.name) == ["Receipts", "Travel"])
        #expect(found.map(\.vaultID) == [home.id, work.id])
        #expect(found.map(\.vaultName) == ["Home", "Work"])

        let looked = try await catalog(root).lookup([VaultScopedID(travel.id, vaultID: work.id)]) {
            try await $0.tags(ids: $1)
        }
        #expect(looked.map(\.value.name) == ["Travel"])
        #expect(looked.map(\.vaultID) == [work.id])
    }

    @Test func aSingleVaultNeedsNoLabel() async throws {
        let service = MockFolioService(tags: [receipts])
        let found = try await catalog(service).acrossVaults { try await $0.tags() }
        #expect(found.map(\.vaultName) == [nil])
        #expect(found.map(\.vaultID) == [MockFolioService.defaultVault.id])
    }
}

@MainActor struct TagUndoTests {
    @Test func undoAppliesTheRevertedChange() async throws {
        let manager = UndoManager()
        manager.groupsByEvent = false
        let change = TagChange(.add, tagID: receipts.id, documents: [DocumentEntityID(documentID: UUID())])
        let (reverts, continuation) = AsyncStream.makeStream(of: TagChange.self)

        manager.beginUndoGrouping()
        TagUndo.register(change, with: manager) { _ = continuation.yield($0) }
        manager.endUndoGrouping()
        #expect(manager.undoActionName == "Add Tag")
        manager.undo()

        let reverted = await reverts.first { _ in true }
        #expect(reverted == change.reverted)
    }

    @Test func aChangeThatAlteredNothingIsNotOffered() {
        let manager = UndoManager()
        manager.groupsByEvent = false
        manager.beginUndoGrouping()
        TagUndo.register(TagChange(.remove, tagID: receipts.id, documents: []), with: manager) { _ in }
        manager.endUndoGrouping()
        #expect(!manager.canUndo)
    }
}
