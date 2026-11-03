import Foundation

public enum DocumentEntryKind: String, Codable, CaseIterable, Sendable {
    case note, comment

    public var title: String { self == .note ? "Notes" : "Comments" }
    public var singular: String { rawValue }
}

public struct DocumentEntry: Identifiable, Decodable, Equatable, Sendable {
    public let id: UUID
    public let documentID: UUID
    public let vaultID: UUID
    public let kind: DocumentEntryKind
    public let body: String
    public let createdAt: Date
    public let updatedAt: Date
    public let createdBy: String?
    public let canEdit: Bool
    public let canDelete: Bool

    enum CodingKeys: String, CodingKey {
        case id, kind, body
        case documentID = "document_id", vaultID = "vault_id"
        case createdAt = "created_at", updatedAt = "updated_at", createdBy = "created_by"
        case canEdit = "can_edit", canDelete = "can_delete"
    }
}

public struct DocumentEntryPage: Decodable, Sendable {
    public let entries: [DocumentEntry]
    public let nextCursor: String?
    public let canCreate: Bool

    enum CodingKeys: String, CodingKey {
        case entries
        case nextCursor = "next_cursor", canCreate = "can_create"
    }
}

public protocol DocumentEntryService: Sendable {
    func entryPage(documentID: UUID, kind: DocumentEntryKind, cursor: String?) async throws -> DocumentEntryPage
    func createEntry(documentID: UUID, kind: DocumentEntryKind, body: String, key: UUID) async throws -> DocumentEntry
    func updateEntry(documentID: UUID, entryID: UUID, body: String) async throws -> DocumentEntry
    func deleteEntry(documentID: UUID, entryID: UUID) async throws
}

public extension DocumentEntryService {
    func entryPage(documentID: UUID, kind: DocumentEntryKind, cursor: String?) async throws -> DocumentEntryPage {
        throw FolioError.server(status: 501, message: "Notes and comments are unavailable.")
    }
    func createEntry(documentID: UUID, kind: DocumentEntryKind, body: String, key: UUID) async throws -> DocumentEntry {
        throw FolioError.server(status: 501, message: "Notes and comments are unavailable.")
    }
    func updateEntry(documentID: UUID, entryID: UUID, body: String) async throws -> DocumentEntry {
        throw FolioError.server(status: 501, message: "Notes and comments are unavailable.")
    }
    func deleteEntry(documentID: UUID, entryID: UUID) async throws {
        throw FolioError.server(status: 501, message: "Notes and comments are unavailable.")
    }
}
