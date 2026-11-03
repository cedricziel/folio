import FolioKit
import Foundation
import Observation

@MainActor @Observable final class ClassificationFixtureService: FolioService {
    private static let vaultID = UUID(uuidString: "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff")!
    private static let retainedTagID = UUID(uuidString: "00000000-0000-4000-8000-000000000001")!
    private static let replacementTagID = UUID(uuidString: "00000000-0000-4000-8000-000000000002")!
    private static let mutationTime = "2026-11-02T10:15:00Z"

    private var entries: [String: [StoredEntry]]
    private let readOnly: Bool
    private var failInitialLoad: Bool
    private(set) var mutationNote = "Isolated classification fixture"

    init(readOnly: Bool, failInitialLoad: Bool, longNames: Bool) {
        self.readOnly = readOnly
        self.failInitialLoad = failInitialLoad
        entries = Dictionary(
            uniqueKeysWithValues: TaxonomyKind.allCases.enumerated().map { offset, kind in
                let name: String
                switch kind {
                case .tags: name = longNames ? "Insurance and household correspondence" : "Insurance"
                case .correspondents: name = "City utilities"
                case .documentTypes: name = "Invoice"
                }
                let original = StoredEntry(
                    id: Self.entryID(offset * 2 + 1), name: name,
                    createdAt: "2025-03-14T10:15:30Z", updatedAt: "2026-10-15T08:30:00Z",
                    deletedAt: "2026-10-15T08:30:00Z", createdBy: "Taylor", updatedBy: "Morgan"
                )
                let replacement = StoredEntry(
                    id: Self.entryID(offset * 2 + 2), name: name,
                    createdAt: "2026-10-20T09:00:00Z", updatedAt: "2026-10-21T09:30:00Z",
                    deletedAt: nil, createdBy: "Morgan", updatedBy: "Jamie"
                )
                return (kind.rawValue, [original, replacement])
            })
    }

    func taxonomy(_ kind: TaxonomyKind, deleted: Bool) async throws -> [TaxonomyEntry] {
        if failInitialLoad {
            failInitialLoad = false
            throw FolioError.unreachable
        }
        return try entries[kind.rawValue, default: []]
            .filter { ($0.deletedAt != nil) == deleted }
            .sorted { $0.id.uuidString < $1.id.uuidString }
            .map { try $0.model() }
    }

    func saveTaxonomy(_ kind: TaxonomyKind, id: UUID?, name: String) async throws {
        try requireWrite()
        var values = entries[kind.rawValue, default: []]
        guard !values.contains(where: { $0.id != id && $0.deletedAt == nil && $0.name == name }) else {
            throw FolioError.server(status: 409, message: "An active entry already uses this name.")
        }
        if let id {
            guard let index = values.firstIndex(where: { $0.id == id && $0.deletedAt == nil }) else {
                throw FolioError.server(status: 404, message: "Classification not found.")
            }
            values[index].name = name
            values[index].updatedAt = Self.mutationTime
            values[index].updatedBy = "Fixture editor"
        } else {
            values.append(
                StoredEntry(
                    id: UUID(), name: name, createdAt: Self.mutationTime,
                    updatedAt: Self.mutationTime, deletedAt: nil,
                    createdBy: "Fixture editor", updatedBy: "Fixture editor"
                ))
        }
        entries[kind.rawValue] = values
        mutationNote = "Saved \(name)"
    }

    func deleteTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws {
        try requireWrite()
        var values = entries[kind.rawValue, default: []]
        guard let index = values.firstIndex(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: "Classification not found.")
        }
        values[index].deletedAt = Self.mutationTime
        values[index].updatedAt = Self.mutationTime
        values[index].updatedBy = "Fixture editor"
        entries[kind.rawValue] = values
        mutationNote =
            id == Self.replacementTagID
            ? "Replacement Insurance moved to Trash" : "Moved \(values[index].name) to Trash"
    }

    func restoreTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws {
        try requireWrite()
        var values = entries[kind.rawValue, default: []]
        guard let index = values.firstIndex(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: "Classification not found.")
        }
        guard !values.contains(where: { $0.id != id && $0.deletedAt == nil && $0.name == values[index].name }) else {
            mutationNote = "Restore conflict retained the original identity"
            throw FolioError.server(status: 409, message: "An active entry already uses this name.")
        }
        values[index].deletedAt = nil
        values[index].updatedAt = Self.mutationTime
        values[index].updatedBy = "Fixture editor"
        entries[kind.rawValue] = values
        mutationNote = id == Self.retainedTagID ? "Original Insurance restored" : "Restored \(values[index].name)"
    }

    private func requireWrite() throws {
        if readOnly { throw FolioError.server(status: 403, message: "This vault is read-only.") }
    }

    private static func entryID(_ number: Int) -> UUID {
        UUID(uuidString: String(format: "00000000-0000-4000-8000-%012d", number))!
    }

    func vaults() async throws -> VaultCatalog {
        VaultCatalog(
            vaults: [Vault(id: Self.vaultID, name: "Fixture", role: readOnly ? .viewer : .editor)],
            defaultVaultID: Self.vaultID)
    }
    nonisolated func scoped(to vaultID: UUID) -> any FolioService { self }
    func ping() async throws {}
    nonisolated func scoped(to vaultID: UUID) -> any FolioService { self }
    func savedViews(after: UUID?) async throws -> [SavedView] { [] }
    func savedView(id: UUID) async throws -> SavedView { throw FolioError.unreadableResponse }
    func saveView(_ draft: SavedViewDraft) async throws -> SavedView { throw FolioError.unreadableResponse }
    func updateSavedView(id: UUID, draft: SavedViewDraft) async throws -> SavedView {
        throw FolioError.unreadableResponse
    }
    func deleteSavedView(id: UUID) async throws { throw FolioError.unreadableResponse }
    func documentPage(query: DocumentBrowseQuery, cursor: String?) async throws -> DocumentPage {
        DocumentPage(vaultID: Self.vaultID, documents: [])
    }
    func documents(matching query: String) async throws -> [Document] { [] }
    func documents(filter: DocumentFilter) async throws -> [Document] { [] }
    func tags() async throws -> [Tag] { [] }
    func correspondents() async throws -> [Correspondent] { [] }
    func documentTypes() async throws -> [DocumentType] { [] }
    func thumbnail(id: Document.ID) async throws -> Data? { nil }
    func document(id: Document.ID) async throws -> Document { throw FolioError.unreadableResponse }
    func deleteDocument(id: Document.ID) async throws { throw FolioError.unreadableResponse }
    func restoreDocument(id: Document.ID) async throws -> Document { throw FolioError.unreadableResponse }
    func downloadFile(for document: Document) async throws -> URL { throw FolioError.unreadableResponse }
    func upload(_ file: UploadFile) async throws -> Document { throw FolioError.unreadableResponse }
    func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws { throw FolioError.unreadableResponse }
    func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws { throw FolioError.unreadableResponse }

    private struct StoredEntry: Encodable {
        let vaultID = UUID(uuidString: "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff")!
        let id: UUID
        var name: String
        var createdAt: String
        var updatedAt: String
        var deletedAt: String?
        var createdBy: String?
        var updatedBy: String?

        private enum CodingKeys: String, CodingKey {
            case id, name
            case vaultID = "vault_id"
            case createdAt = "created_at", updatedAt = "updated_at", deletedAt = "deleted_at"
            case createdBy = "created_by", updatedBy = "updated_by"
        }

        func model() throws -> TaxonomyEntry {
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            return try decoder.decode(TaxonomyEntry.self, from: JSONEncoder().encode(self))
        }
    }
}
