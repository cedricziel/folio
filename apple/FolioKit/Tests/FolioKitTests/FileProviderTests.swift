import Foundation
import Testing

@testable import FolioKit

private let personID = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!
private let typeID = UUID(uuidString: "66666666-7777-4888-9999-000000000000")!
private let tagID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!

struct FolioItemIDTests {
    @Test(arguments: [
        FolioItemID.folder(.root),
        .folder(.inbox),
        .folder(.all),
        .folder(.tag(tagID)),
        .folder(.correspondent(personID)),
        .folder(.type(typeID)),
        .document(personID, in: .inbox),
        .document(typeID, in: .correspondent(personID)),
    ])
    func rawValueRoundTrips(id: FolioItemID) {
        #expect(FolioItemID(rawValue: id.rawValue) == id)
    }

    @Test func rejectsGarbage() {
        #expect(FolioItemID(rawValue: "nonsense") == nil)
        #expect(FolioItemID(rawValue: "doc:not-a-uuid@inbox") == nil)
        #expect(FolioItemID(rawValue: "tag:") == nil)
    }

    @Test func parentChainWalksUpToTheRoot() {
        let id = FolioItemID.document(typeID, in: .correspondent(personID))
        #expect(
            id.ancestors == [.folder(.correspondent(personID)), .folder(.correspondents), .folder(.root)]
        )
        #expect(FolioItemID.folder(.root).ancestors.isEmpty)
        #expect(FolioItemID.folder(.inbox).parent == .folder(.root))
    }
}

struct FolioCatalogTests {
    private func document(
        _ title: String, filename: String? = nil, person: UUID? = nil, type: UUID? = nil,
        read: Bool = true
    ) -> Document {
        Document(
            title: title, content: read ? "text" : nil, originalFilename: filename,
            mimeType: "application/pdf", correspondentID: person, documentTypeID: type
        )
    }

    @Test func rootListsTheVirtualFolders() async throws {
        let catalog = FolioCatalog(service: MockFolioService())
        let names = try await catalog.children(of: .root).map(\.name)
        #expect(names == ["Inbox", "All documents", "Tags", "Correspondents", "Types"])
    }

    @Test func inboxHoldsOnlyUnfiledReadDocuments() async throws {
        let unfiled = document("Letter")
        let filed = document("Bill", person: personID)
        let reading = document("Scan", read: false)
        let catalog = FolioCatalog(service: MockFolioService(documents: [unfiled, filed, reading]))
        let inbox = try await catalog.children(of: .inbox)
        #expect(inbox.map(\.name) == ["Letter"])
        #expect(inbox[0].id == .document(unfiled.id, in: .inbox))
        #expect(inbox[0].parent == .folder(.inbox))
    }

    @Test func allDocumentsListsEverything() async throws {
        let catalog = FolioCatalog(
            service: MockFolioService(documents: [document("A"), document("B", person: personID)])
        )
        #expect(try await catalog.children(of: .all).count == 2)
    }

    @Test func correspondentFolderFiltersOnTheServer() async throws {
        let mine = document("Bill", person: personID)
        let service = MockFolioService(
            documents: [mine, document("Other")],
            correspondents: [Correspondent(id: personID, name: "Stadtwerke/Köln")]
        )
        let catalog = FolioCatalog(service: service)

        let folders = try await catalog.children(of: .correspondents)
        #expect(folders.map(\.name) == ["Stadtwerke-Köln"])
        #expect(folders[0].isFolder)
        #expect(folders[0].id == .folder(.correspondent(personID)))

        let docs = try await catalog.children(of: .correspondent(personID))
        #expect(docs.map(\.id) == [.document(mine.id, in: .correspondent(personID))])
    }

    @Test func typeFoldersListTypes() async throws {
        let service = MockFolioService(types: [DocumentType(id: typeID, name: "Invoice")])
        let folders = try await FolioCatalog(service: service).children(of: .types)
        #expect(folders.map(\.name) == ["Invoice"])
    }

    @Test func tagFoldersListTagsAndStayEmptyUntilTheAPIReportsTags() async throws {
        let service = MockFolioService(documents: [document("A")])
        await service.setTags([Tag(id: tagID, name: "Insurance")])
        let catalog = FolioCatalog(service: service)
        #expect(try await catalog.children(of: .tags).map(\.name) == ["Insurance"])
        #expect(try await catalog.children(of: .tag(tagID)).isEmpty)
    }

    @Test func documentsKeepTheirFilenameAndDisambiguateCollisions() async throws {
        let first = document("Bill", filename: "bill.pdf")
        let second = document("Bill again", filename: "bill.pdf")
        let titled = document("No file name")
        let catalog = FolioCatalog(service: MockFolioService(documents: [first, second, titled]))
        let names = try await catalog.children(of: .all).map(\.name)
        #expect(names.contains("No file name"))
        #expect(names.filter { $0.hasPrefix("bill") }.count == 2)
        #expect(Set(names).count == 3)
    }

    @Test func nodeLookupResolvesFoldersAndDocuments() async throws {
        let doc = document("Letter", filename: "letter.pdf")
        let service = MockFolioService(
            documents: [doc], correspondents: [Correspondent(id: personID, name: "Stadtwerke")]
        )
        let catalog = FolioCatalog(service: service)

        let folder = try #require(try await catalog.node(for: .folder(.correspondent(personID))))
        #expect(folder.name == "Stadtwerke")
        #expect(folder.parent == .folder(.correspondents))

        let node = try #require(try await catalog.node(for: .document(doc.id, in: .inbox)))
        #expect(node.name == "letter.pdf")
        #expect(node.contentType == "application/pdf")

        #expect(try await catalog.node(for: .document(UUID(), in: .all)) == nil)
    }

    @Test func workingSetAnchorChangesWhenADocumentChanges() async throws {
        var doc = document("Letter")
        let before = try await FolioCatalog(service: MockFolioService(documents: [doc])).changeToken()
        doc.updatedAt = doc.updatedAt.addingTimeInterval(60)
        let after = try await FolioCatalog(service: MockFolioService(documents: [doc])).changeToken()
        #expect(before != after)
        let same = try await FolioCatalog(service: MockFolioService(documents: [doc])).changeToken()
        #expect(after == same)
    }
}
