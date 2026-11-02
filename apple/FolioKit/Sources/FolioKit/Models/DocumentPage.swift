import Foundation

public struct DocumentBrowseQuery: Equatable, Sendable {
    public enum Sort: String, CaseIterable, Sendable {
        case newest, oldest, title
    }

    public var text: String
    public var filter: DocumentFilter
    public var sort: Sort
    public var limit: Int32

    public init(text: String = "", filter: DocumentFilter = .init(), sort: Sort = .newest, limit: Int32 = 50) {
        self.text = text
        self.filter = filter
        self.sort = sort
        self.limit = limit
    }
}

public struct DocumentPage: Equatable, Sendable {
    public let vaultID: UUID
    public let documents: [Document]
    public let nextCursor: String?

    public init(vaultID: UUID, documents: [Document], nextCursor: String? = nil) {
        self.vaultID = vaultID
        self.documents = documents
        self.nextCursor = nextCursor
    }
}
