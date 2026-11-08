import Foundation
import Testing

@testable import FolioKit

private let personID = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!
private let typeID = UUID(uuidString: "66666666-7777-4888-9999-000000000000")!
private let tagID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!

struct FolioItemIDTests {
    @Test func scopedDocumentIdentifiersRetainVaultAndParents() {
        let id = FolioItemID.document(typeID, in: .scoped(tagID, .correspondent(personID)))
        #expect(
            id.rawValue
                == "doc:66666666-7777-4888-9999-000000000000@vault:aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee/correspondent:11111111-2222-4333-8444-555555555555"
        )
        #expect(FolioItemID(rawValue: id.rawValue) == id)
        #expect(
            id.ancestors == [
                .folder(.scoped(tagID, .correspondent(personID))), .folder(.scoped(tagID, .correspondents)),
                .folder(.vault(tagID)), .folder(.root),
            ])
        #expect(FolioItemID(rawValue: "vault:\(tagID)/vault:\(personID)") == nil)
    }

    @Test(arguments: [
        FolioItemID.folder(.root),
        .folder(.vault(tagID)),
        .folder(.scoped(tagID, .inbox)),
        .folder(.scoped(tagID, .all)),
        .folder(.scoped(tagID, .tag(tagID))),
        .folder(.scoped(tagID, .correspondent(personID))),
        .folder(.scoped(tagID, .type(typeID))),
        .document(personID, in: .scoped(tagID, .inbox)),
        .document(typeID, in: .scoped(tagID, .correspondent(personID))),
    ])
    func rawValueRoundTrips(id: FolioItemID) {
        #expect(FolioItemID(rawValue: id.rawValue) == id)
    }

    @Test func rejectsGarbage() {
        #expect(FolioItemID(rawValue: "nonsense") == nil)
        #expect(FolioItemID(rawValue: "doc:not-a-uuid@inbox") == nil)
        #expect(FolioItemID(rawValue: "tag:") == nil)
    }

    @Test(arguments: [
        "inbox", "all", "tags", "correspondents", "types",
        "tag:aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        "doc:66666666-7777-4888-9999-000000000000@inbox",
    ])
    func rejectsUnscopedIdentifiers(identifier: String) {
        #expect(FolioItemID(rawValue: identifier) == nil)
    }

    @Test func parentChainWalksUpToTheRoot() {
        let id = FolioItemID.document(typeID, in: .scoped(tagID, .correspondent(personID)))
        #expect(
            id.ancestors == [
                .folder(.scoped(tagID, .correspondent(personID))), .folder(.scoped(tagID, .correspondents)),
                .folder(.vault(tagID)), .folder(.root),
            ]
        )
        #expect(FolioItemID.folder(.root).ancestors.isEmpty)
        #expect(FolioItemID.folder(.scoped(tagID, .inbox)).parent == .folder(.vault(tagID)))
    }
}

struct VaultProviderCatalogTests {
    private let personal = Vault(id: UUID(), name: "Personal", role: .owner)
    private let shared = Vault(id: UUID(), name: "Household", role: .viewer)

    @Test func rootListsVaultsAndEachLibraryKeepsItsOwnDocuments() async throws {
        let privateDocument = Document(title: "Private letter")
        let sharedDocument = Document(title: "Shared letter")
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id),
            vaultServices: [
                personal.id: MockFolioService(documents: [privateDocument]),
                shared.id: MockFolioService(documents: [sharedDocument]),
            ]
        )
        let catalog = FolioCatalog(service: source)

        let roots = try await catalog.children(of: .root)
        let privateFiles = try await catalog.children(of: .scoped(personal.id, .all))
        let sharedFiles = try await catalog.children(of: .scoped(shared.id, .all))

        #expect(Set(roots.map(\.id)) == [.folder(.vault(personal.id)), .folder(.vault(shared.id))])
        #expect(privateFiles.map(\.id) == [.document(privateDocument.id, in: .scoped(personal.id, .all))])
        #expect(sharedFiles.map(\.id) == [.document(sharedDocument.id, in: .scoped(shared.id, .all))])
        #expect(
            try await catalog.children(of: .vault(shared.id)).map(\.parent).allSatisfy {
                $0 == .folder(.vault(shared.id))
            })
    }

    @Test func collidingVaultNamesUseStableUUIDSuffixesIndependentOfOrder() async throws {
        let first = Vault(id: personal.id, name: "Home/Archive", role: .owner)
        let second = Vault(id: shared.id, name: "home:archive", role: .viewer)
        let source = MockFolioService(vaults: VaultCatalog(vaults: [first, second], defaultVaultID: first.id))
        let catalog = FolioCatalog(service: source)
        let before = try await catalog.children(of: .root)
        await source.setVaults(VaultCatalog(vaults: [second, first], defaultVaultID: second.id))
        let after = try await catalog.children(of: .root)

        #expect(before == after)
        #expect(Set(before.map { $0.name.lowercased() }).count == 2)
        #expect(
            before.first { $0.id == .folder(.vault(first.id)) }?.name
                == "Home-Archive (\(first.id.uuidString.lowercased()))")
    }

    @Test func foldersCarryTheDatesOfWhatTheyStandFor() async throws {
        let vaultCreated = Date(timeIntervalSince1970: 1_700_000_000)
        let personCreated = Date(timeIntervalSince1970: 1_710_000_000)
        let vault = Vault(id: UUID(), name: "Personal", role: .owner, createdAt: vaultCreated)
        let person = Correspondent(name: "ACME", createdAt: personCreated)
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [vault], defaultVaultID: vault.id),
            vaultServices: [vault.id: MockFolioService(correspondents: [person])]
        )
        let catalog = FolioCatalog(service: source)

        let root = try #require(try await catalog.children(of: .root).first)
        let sections = try await catalog.children(of: .vault(vault.id))
        let people = try await catalog.children(of: .scoped(vault.id, .correspondents))

        #expect(root.created == vaultCreated)
        #expect(!sections.isEmpty && sections.allSatisfy { $0.created == vaultCreated && $0.modified == vaultCreated })
        #expect(people.map(\.created) == [personCreated])
        #expect(try await catalog.allNodes().allSatisfy { $0.created != nil })
    }

    @Test func viewerInboxIsReadOnlyAndCannotUpload() async throws {
        let target = MockFolioService()
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [shared], defaultVaultID: shared.id), vaultServices: [shared.id: target]
        )
        let catalog = FolioCatalog(service: source)
        let inbox = try #require(try await catalog.node(for: .folder(.scoped(shared.id, .inbox))))

        #expect(!inbox.canUpload)
        await #expect(throws: FolioError.server(status: 403, message: nil)) {
            _ = try await catalog.upload(
                UploadFile(data: Data(), filename: "a.pdf", mimeType: "application/pdf"), to: .scoped(shared.id, .inbox)
            )
        }
        #expect(await target.uploads.isEmpty)
    }

    @Test func uploadTargetsItsVaultInboxAndUnscopedIdentifiersAreRejected() async throws {
        let target = MockFolioService()
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal], defaultVaultID: personal.id), vaultServices: [personal.id: target]
        )
        let catalog = FolioCatalog(service: source)
        let file = UploadFile(data: Data(), filename: "letter.pdf", mimeType: "application/pdf")

        let uploaded = try await catalog.upload(file, to: .scoped(personal.id, .inbox))

        #expect(uploaded.parent == .folder(.scoped(personal.id, .inbox)))
        #expect(await target.uploads == [file])
        #expect(await source.uploads.isEmpty)
        #expect(try await catalog.node(for: .folder(.inbox)) == nil)
        #expect(try await catalog.children(of: .all).isEmpty)
        await #expect(throws: FolioError.server(status: 404, message: nil)) {
            _ = try await catalog.upload(file, to: .inbox)
        }
    }

    @Test func revokedVaultDoesNotBlockUnrelatedWorkingSetAndNetworkFailureDoesNotDeleteIt() async throws {
        let availableDocument = Document(title: "Available")
        let revoked = MockFolioService()
        let available = MockFolioService(documents: [availableDocument])
        await revoked.setFailure(.server(status: 404, message: nil))
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id),
            vaultServices: [personal.id: available, shared.id: revoked]
        )
        let catalog = FolioCatalog(service: source)

        let nodes = try await catalog.allNodes()

        #expect(nodes.contains { $0.id == .document(availableDocument.id, in: .scoped(personal.id, .all)) })
        #expect(!nodes.contains { $0.id == .folder(.vault(shared.id)) })
        await revoked.setFailure(.unreachable)
        await #expect(throws: FolioError.unreachable) { _ = try await catalog.allNodes() }
    }
}

struct FolioCatalogTests {
    @Test func emptyDiscoveryDoesNotExposeUnscopedDocuments() async throws {
        let source = MockFolioService(documents: [Document(title: "Hidden")], vaults: .empty)
        let catalog = FolioCatalog(service: source)

        #expect(try await catalog.children(of: .root).isEmpty)
        #expect(try await catalog.children(of: .all).isEmpty)
        #expect(try await catalog.allNodes().isEmpty)
    }

    private func document(
        _ title: String, filename: String? = nil, person: UUID? = nil, type: UUID? = nil,
        read: Bool = true
    ) -> Document {
        Document(
            title: title, content: read ? "text" : nil, originalFilename: filename,
            mimeType: "application/pdf", correspondentID: person, documentTypeID: type,
            ocrStatus: read ? .done : .pending, embedStatus: .skipped
        )
    }

    @Test func vaultListsTheVirtualFolders() async throws {
        let catalog = FolioCatalog(service: MockFolioService())
        let names = try await catalog.children(of: .vault(MockFolioService.defaultVault.id)).map(\.name)
        #expect(names == ["Inbox", "All documents", "Tags", "Correspondents", "Types"])
    }

    @Test func inboxIncludesUploadsBeforeProcessingFinishes() async throws {
        let unfiled = document("Letter")
        let filed = document("Bill", person: personID)
        let reading = document("Scan", read: false)
        let catalog = FolioCatalog(service: MockFolioService(documents: [unfiled, filed, reading]))
        let inbox = try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .inbox))
        #expect(Set(inbox.map(\.name)) == ["Letter", "Scan"])
        #expect(
            inbox.contains { $0.id == .document(unfiled.id, in: .scoped(MockFolioService.defaultVault.id, .inbox)) })
        #expect(inbox.allSatisfy { $0.parent == .folder(.scoped(MockFolioService.defaultVault.id, .inbox)) })
    }

    @Test func allDocumentsListsEverything() async throws {
        let catalog = FolioCatalog(
            service: MockFolioService(documents: [document("A"), document("B", person: personID)])
        )
        #expect(try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .all)).count == 2)
    }

    @Test func correspondentFolderFiltersOnTheServer() async throws {
        let mine = document("Bill", person: personID)
        let service = MockFolioService(
            documents: [mine, document("Other")],
            correspondents: [Correspondent(id: personID, name: "Stadtwerke/Köln")]
        )
        let catalog = FolioCatalog(service: service)

        let folders = try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .correspondents))
        #expect(folders.map(\.name) == ["Stadtwerke-Köln"])
        #expect(folders[0].isFolder)
        #expect(folders[0].id == .folder(.scoped(MockFolioService.defaultVault.id, .correspondent(personID))))

        let docs = try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .correspondent(personID)))
        #expect(
            docs.map(\.id) == [
                .document(mine.id, in: .scoped(MockFolioService.defaultVault.id, .correspondent(personID)))
            ])
    }

    @Test func typeFoldersListTypes() async throws {
        let service = MockFolioService(types: [DocumentType(id: typeID, name: "Invoice")])
        let folders = try await FolioCatalog(service: service).children(
            of: .scoped(MockFolioService.defaultVault.id, .types))
        #expect(folders.map(\.name) == ["Invoice"])
    }

    @Test func tagFoldersListTagsAndTheirDocuments() async throws {
        let insurance = Tag(id: tagID, name: "Insurance")
        let tagged = Document(title: "Policy", originalFilename: "policy.pdf", tags: [insurance])
        let service = MockFolioService(documents: [document("A"), tagged])
        await service.setTags([insurance])
        let catalog = FolioCatalog(service: service)
        #expect(
            try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .tags)).map(\.name) == [
                "Insurance"
            ])
        #expect(
            try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .tag(tagID))).map(\.name) == [
                "policy.pdf"
            ])
        #expect(await service.filters.last == DocumentFilter(tagIDs: [tagID]))
    }

    @Test func documentsKeepTheirFilenameAndDisambiguateCollisions() async throws {
        let first = document("Bill", filename: "bill.pdf")
        let second = document("Bill again", filename: "bill.pdf")
        let titled = document("No file name")
        let catalog = FolioCatalog(service: MockFolioService(documents: [first, second, titled]))
        let names = try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .all)).map(\.name)
        #expect(names.contains("No file name"))
        #expect(names.filter { $0.hasPrefix("bill") }.count == 2)
        #expect(Set(names).count == 3)
    }

    @Test func documentSuffixesSkipNamesAlreadyTaken() async throws {
        let first = Document(
            id: UUID(uuidString: "00000000-0000-4000-8000-000000000001")!, title: "Bill", originalFilename: "bill.pdf")
        let literal = Document(
            id: UUID(uuidString: "11111111-0000-4000-8000-000000000001")!, title: "Bill",
            originalFilename: "bill (aaaa).pdf")
        let second = Document(
            id: UUID(uuidString: "AAAA0000-0000-4000-8000-000000000001")!, title: "Bill", originalFilename: "Bill.pdf")
        let catalog = FolioCatalog(service: MockFolioService(documents: [second, literal, first]))

        let names = try await catalog.children(of: .scoped(MockFolioService.defaultVault.id, .all)).map(\.name)

        #expect(Set(names.map { $0.lowercased() }).count == 3)
        #expect(names.contains("bill.pdf"))
        #expect(names.contains("bill (aaaa).pdf"))
    }

    @Test func taxonomyFoldersWhoseNamesSanitizeAlikeGetDistinctNames() async throws {
        let slash = Correspondent(id: UUID(uuidString: "00000000-0000-4000-8000-000000000001")!, name: "A/B")
        let colon = Correspondent(id: UUID(uuidString: "00000000-0000-4000-8000-000000000002")!, name: "A:B")
        let service = MockFolioService(correspondents: [colon, slash])
        await service.setTags([Tag(name: "Tax"), Tag(name: "tax")])
        let catalog = FolioCatalog(service: service)
        let vault = MockFolioService.defaultVault.id

        let people = try await catalog.children(of: .scoped(vault, .correspondents)).map(\.name)
        let tags = try await catalog.children(of: .scoped(vault, .tags)).map(\.name)
        #expect(people.first == "A-B")
        #expect(Set(people.map { $0.lowercased() }).count == 2)
        #expect(Set(tags.map { $0.lowercased() }).count == 2)

        let workingSet = try await catalog.allNodes()
        let siblings = workingSet.filter { $0.parent == .folder(.scoped(vault, .correspondents)) }.map(\.name)
        #expect(Set(siblings) == Set(people))
    }

    @Test func uniqueNameFallsBackToTheFullIdentifier() {
        let id = UUID(uuidString: "ABCD0000-0000-4000-8000-000000000001")!
        var taken: Set<String> = ["bill.pdf", "bill (abcd).pdf"]

        let name = FolioCatalog.uniqueName("Bill.pdf", id: id, taken: &taken)

        #expect(name == "Bill (abcd0000-0000-4000-8000-000000000001).pdf")
        #expect(taken.contains(name.lowercased()))
    }

    @Test func nodeLookupResolvesFoldersAndDocuments() async throws {
        let doc = document("Letter", filename: "letter.pdf")
        let service = MockFolioService(
            documents: [doc], correspondents: [Correspondent(id: personID, name: "Stadtwerke")]
        )
        let catalog = FolioCatalog(service: service)

        let folder = try #require(
            try await catalog.node(for: .folder(.scoped(MockFolioService.defaultVault.id, .correspondent(personID)))))
        #expect(folder.name == "Stadtwerke")
        #expect(folder.parent == .folder(.scoped(MockFolioService.defaultVault.id, .correspondents)))

        let node = try #require(
            try await catalog.node(for: .document(doc.id, in: .scoped(MockFolioService.defaultVault.id, .inbox))))
        #expect(node.name == "letter.pdf")
        #expect(node.contentType == "application/pdf")

        #expect(
            try await catalog.node(for: .document(UUID(), in: .scoped(MockFolioService.defaultVault.id, .all))) == nil)
    }

    @Test func workingSetIncludesEveryDocumentAliasAndItsParents() async throws {
        let tag = Tag(id: tagID, name: "Taxes")
        let doc = Document(title: "Letter", correspondentID: personID, documentTypeID: typeID, tags: [tag])
        let service = MockFolioService(
            documents: [doc],
            correspondents: [Correspondent(id: personID, name: "Sender")],
            types: [DocumentType(id: typeID, name: "Letter")]
        )
        await service.setTags([tag])
        let nodes = try await FolioCatalog(service: service).allNodes()
        let ids = Set(nodes.map(\.id))
        #expect(ids.contains(.document(doc.id, in: .scoped(MockFolioService.defaultVault.id, .all))))
        #expect(ids.contains(.document(doc.id, in: .scoped(MockFolioService.defaultVault.id, .inbox))))
        #expect(
            ids.contains(.document(doc.id, in: .scoped(MockFolioService.defaultVault.id, .correspondent(personID)))))
        #expect(ids.contains(.document(doc.id, in: .scoped(MockFolioService.defaultVault.id, .type(typeID)))))
        #expect(ids.contains(.document(doc.id, in: .scoped(MockFolioService.defaultVault.id, .tag(tagID)))))
        #expect(nodes.allSatisfy { $0.parent == .folder(.root) || ids.contains($0.parent) })
    }
}
