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
    public var ocrStatus: ProcessingStatus
    public var embedStatus: ProcessingStatus
    /// Only the list and get endpoints report tags; search results and uploads come back without them.
    public var tags: [Tag]
    public var documentDate: DocumentDate?
    public var canEditDocumentDate: Bool
    /// Nil identifies servers that predate explicit review.
    public var reviewed: Bool?
    public var reviewedAt: Date?
    public var reviewedBy: String?
    public var reviewOrigin: String?
    public var canReviewDocument: Bool
    public var createdAt: Date
    public var updatedAt: Date
    public var createdBy: String?
    public var updatedBy: String?
    public var deletedAt: Date?

    public init(
        id: UUID = UUID(),
        title: String,
        content: String? = nil,
        originalFilename: String? = nil,
        mimeType: String? = nil,
        correspondentID: UUID? = nil,
        documentTypeID: UUID? = nil,
        pageCount: Int? = nil,
        ocrStatus: ProcessingStatus = .pending,
        embedStatus: ProcessingStatus = .pending,
        tags: [Tag] = [],
        documentDate: DocumentDate? = nil,
        canEditDocumentDate: Bool = false,
        reviewed: Bool? = nil,
        reviewedAt: Date? = nil,
        reviewedBy: String? = nil,
        reviewOrigin: String? = nil,
        canReviewDocument: Bool = false,
        createdAt: Date = Date(),
        updatedAt: Date = Date(),
        createdBy: String? = nil,
        updatedBy: String? = nil,
        deletedAt: Date? = nil
    ) {
        self.id = id
        self.title = title
        self.content = content
        self.originalFilename = originalFilename
        self.mimeType = mimeType
        self.correspondentID = correspondentID
        self.documentTypeID = documentTypeID
        self.pageCount = pageCount
        self.ocrStatus = ocrStatus
        self.embedStatus = embedStatus
        self.tags = tags
        self.documentDate = documentDate
        self.canEditDocumentDate = canEditDocumentDate
        self.reviewed = reviewed
        self.reviewedAt = reviewedAt
        self.reviewedBy = reviewedBy
        self.reviewOrigin = reviewOrigin
        self.canReviewDocument = canReviewDocument
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.createdBy = createdBy
        self.updatedBy = updatedBy
        self.deletedAt = deletedAt
    }
}

public extension Document {
    /// State reported by the server for one processing stage.
    enum ProcessingStatus: String, Sendable {
        case pending, running, done, failed, skipped

        var isActive: Bool {
            self == .pending || self == .running
        }
    }

    enum Status: Equatable, Sendable {
        case reading(pages: Int?)
        case indexing
        case failed
        case needsReview
        case filed
    }

    var status: Status {
        if reviewed != nil && (ocrStatus == .failed || embedStatus == .failed) { return .failed }
        if ocrStatus.isActive {
            return .reading(pages: pageCount)
        }
        if ocrStatus == .failed {
            return .failed
        }
        if embedStatus.isActive {
            return .indexing
        }
        if embedStatus == .failed {
            return .failed
        }
        if let reviewed {
            return reviewed ? .filed : .needsReview
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
    public var deletedAt: Date?
    public var createdAt: Date?

    public var displayName: String { deletedAt == nil ? name : "\(name) (deleted)" }

    public init(
        id: UUID = UUID(), name: String, color: String? = nil, deletedAt: Date? = nil, createdAt: Date? = nil
    ) {
        self.id = id
        self.name = name
        self.color = color
        self.deletedAt = deletedAt
        self.createdAt = createdAt
    }
}

public struct Correspondent: Identifiable, Hashable, Sendable {
    public let id: UUID
    public var name: String
    public var createdAt: Date?

    public init(id: UUID = UUID(), name: String, createdAt: Date? = nil) {
        self.id = id
        self.name = name
        self.createdAt = createdAt
    }
}

public struct DocumentType: Identifiable, Hashable, Sendable {
    public let id: UUID
    public var name: String
    public var createdAt: Date?

    public init(id: UUID = UUID(), name: String, createdAt: Date? = nil) {
        self.id = id
        self.name = name
        self.createdAt = createdAt
    }
}

/// What the list endpoint can narrow by. Empty fields do not filter.
public struct DocumentFilter: Equatable, Sendable {
    public var reviewed: Bool?
    public var documentDateFrom: DocumentDate?
    public var documentDateTo: DocumentDate?
    public var requiresReviewSupport: Bool { reviewed != nil || documentDateFrom != nil || documentDateTo != nil }

    public var tagIDs: [Tag.ID]
    public var correspondentID: Correspondent.ID?
    public var documentTypeID: DocumentType.ID?
    /// Inclusive.
    public var createdAfter: Date?
    /// Exclusive.
    public var createdBefore: Date?

    public init(
        tagIDs: [Tag.ID] = [], correspondentID: Correspondent.ID? = nil, documentTypeID: DocumentType.ID? = nil,
        createdAfter: Date? = nil, createdBefore: Date? = nil,
        reviewed: Bool? = nil, documentDateFrom: DocumentDate? = nil, documentDateTo: DocumentDate? = nil
    ) {
        self.tagIDs = tagIDs
        self.correspondentID = correspondentID
        self.documentTypeID = documentTypeID
        self.createdAfter = createdAfter
        self.createdBefore = createdBefore
        self.reviewed = reviewed
        self.documentDateFrom = documentDateFrom
        self.documentDateTo = documentDateTo
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
