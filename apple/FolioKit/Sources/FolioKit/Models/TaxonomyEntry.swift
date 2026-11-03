import Foundation

public enum TaxonomyKind: String, CaseIterable, Identifiable, Sendable {
    case tags, correspondents, documentTypes = "document_types"

    public var id: String { rawValue }
    public var title: String {
        switch self {
        case .tags: "Tags"
        case .correspondents: "From"
        case .documentTypes: "Types"
        }
    }
}

public struct TaxonomyEntry: Decodable, Identifiable, Hashable, Sendable {
    public let id: UUID
    public let vaultID: UUID
    public var name: String
    public let color: String?
    public let createdAt: Date
    public let updatedAt: Date
    public let deletedAt: Date?
    public let createdBy: String?
    public let updatedBy: String?

    private enum CodingKeys: String, CodingKey {
        case id, name, color
        case vaultID = "vault_id", createdAt = "created_at", updatedAt = "updated_at"
        case deletedAt = "deleted_at", createdBy = "created_by", updatedBy = "updated_by"
    }
}
