import Foundation

public struct DocumentBrowseQuery: Equatable, Sendable {
    public enum Sort: String, CaseIterable, Sendable {
        case newest, oldest, title
    }

    public var text: String
    public var filter: DocumentFilter
    public var sort: Sort
    public var limit: Int32
    /// Telemetry only: which list this query serves. Not sent to the server and not part of equality.
    public var scope: BrowseScope

    public init(
        text: String = "", filter: DocumentFilter = .init(), sort: Sort = .newest, limit: Int32 = 50,
        scope: BrowseScope = .all
    ) {
        self.text = text
        self.filter = filter
        self.sort = sort
        self.limit = limit
        self.scope = scope
    }

    public static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.text == rhs.text && lhs.filter == rhs.filter && lhs.sort == rhs.sort && lhs.limit == rhs.limit
    }
}

public struct DocumentPage: Equatable, Sendable {
    public let vaultID: UUID
    public let documents: [Document]
    public let nextCursor: String?
    public let supportsReviewFilters: Bool

    public init(vaultID: UUID, documents: [Document], nextCursor: String? = nil, supportsReviewFilters: Bool = false) {
        self.vaultID = vaultID
        self.documents = documents
        self.nextCursor = nextCursor
        self.supportsReviewFilters = supportsReviewFilters
    }
}
