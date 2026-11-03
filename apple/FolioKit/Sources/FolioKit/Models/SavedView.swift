import Foundation

public struct SavedView: Identifiable, Equatable, Sendable {
    public enum Visibility: String, CaseIterable, Sendable {
        case `private`, shared

        public var title: String {
            self == .private ? "Only me" : "Everyone in this vault"
        }
    }

    public enum Presentation: String, CaseIterable, Sendable {
        case list, grid

        public var title: String {
            self == .list ? "List" : "Grid"
        }
    }

    public let id: UUID
    public let vaultID: UUID
    public let ownerID: String
    public var name: String
    public var visibility: Visibility
    public var query: DocumentBrowseQuery
    public var presentation: Presentation
    public let createdAt: Date
    public let updatedAt: Date
    public let createdBy: String?
    public let updatedBy: String?

    public init(
        id: UUID, vaultID: UUID, ownerID: String, name: String,
        visibility: Visibility = .private, query: DocumentBrowseQuery,
        presentation: Presentation = .list, createdAt: Date, updatedAt: Date,
        createdBy: String?, updatedBy: String?
    ) {
        self.id = id
        self.vaultID = vaultID
        self.ownerID = ownerID
        self.name = name
        self.visibility = visibility
        self.query = query
        self.presentation = presentation
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.createdBy = createdBy
        self.updatedBy = updatedBy
    }
}

public struct SavedViewDraft: Equatable, Sendable {
    public var name: String
    public var visibility: SavedView.Visibility
    public var query: DocumentBrowseQuery {
        didSet { replacesQuery = true }
    }
    public private(set) var replacesQuery = true
    public var presentation: SavedView.Presentation

    public init(
        name: String = "", visibility: SavedView.Visibility = .private,
        query: DocumentBrowseQuery, presentation: SavedView.Presentation = .list
    ) {
        self.name = name
        self.visibility = visibility
        self.query = query
        self.presentation = presentation
    }

    public init(_ view: SavedView) {
        self.init(name: view.name, visibility: view.visibility, query: view.query, presentation: view.presentation)
        replacesQuery = false
    }
}
