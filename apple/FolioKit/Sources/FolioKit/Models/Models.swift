import Foundation

public struct Document: Identifiable, Hashable, Sendable {
    public let id: UUID
    public var title: String
    public var content: String?
    public var originalFilename: String?
    public var mimeType: String?
    public var correspondentID: UUID?
    public var documentTypeID: UUID?
    public var pageCount: Int?
    /// Not in the API response yet; the UI renders them as soon as the server sends them.
    public var tags: [Tag]
    public var createdAt: Date
    public var updatedAt: Date

    public init(
        id: UUID = UUID(),
        title: String,
        content: String? = nil,
        originalFilename: String? = nil,
        mimeType: String? = nil,
        correspondentID: UUID? = nil,
        documentTypeID: UUID? = nil,
        pageCount: Int? = nil,
        tags: [Tag] = [],
        createdAt: Date = Date(),
        updatedAt: Date = Date()
    ) {
        self.id = id
        self.title = title
        self.content = content
        self.originalFilename = originalFilename
        self.mimeType = mimeType
        self.correspondentID = correspondentID
        self.documentTypeID = documentTypeID
        self.pageCount = pageCount
        self.tags = tags
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }
}

public extension Document {
    /// The API has no processing state, so it is inferred: no extracted text means the
    /// server is still reading the file; no correspondent and no type means nobody has filed it.
    enum Status: Equatable, Sendable {
        case reading(pages: Int?)
        case needsReview
        case filed
    }

    var status: Status {
        if content?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true {
            return .reading(pages: pageCount)
        }
        if correspondentID == nil && documentTypeID == nil {
            return .needsReview
        }
        return .filed
    }
}

public struct Tag: Identifiable, Hashable, Sendable {
    public let id: UUID
    public var name: String
    public var color: String?

    public init(id: UUID = UUID(), name: String, color: String? = nil) {
        self.id = id
        self.name = name
        self.color = color
    }
}

public struct Correspondent: Identifiable, Hashable, Sendable {
    public let id: UUID
    public var name: String

    public init(id: UUID = UUID(), name: String) {
        self.id = id
        self.name = name
    }
}

public struct DocumentType: Identifiable, Hashable, Sendable {
    public let id: UUID
    public var name: String

    public init(id: UUID = UUID(), name: String) {
        self.id = id
        self.name = name
    }
}

/// A file picked from Files, Photos, the scanner or the share sheet, ready to send to the server.
public struct UploadFile: Sendable, Equatable {
    public var data: Data
    public var filename: String
    public var mimeType: String
    public var title: String?

    public init(data: Data, filename: String, mimeType: String, title: String? = nil) {
        self.data = data
        self.filename = filename
        self.mimeType = mimeType
        self.title = title
    }
}

/// Tag fills from design/README.md, in assignment order.
public enum TagTint: CaseIterable, Sendable {
    case sage, slate, clay, ochre, plum, sky

    /// A tag keeps its tint for its lifetime, so it is derived from the name rather than list position.
    public static func tint(forTagNamed name: String) -> TagTint {
        let hash = name.lowercased().unicodeScalars.reduce(UInt32(5381)) { ($0 &* 33) &+ $1.value }
        return allCases[Int(hash % UInt32(allCases.count))]
    }
}
