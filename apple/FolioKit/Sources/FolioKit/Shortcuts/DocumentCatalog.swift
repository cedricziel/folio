import Foundation

/// A document as Shortcuts, Siri and Spotlight see it: names instead of ids.
public struct DocumentSummary: Identifiable, Hashable, Sendable {
    public let id: Document.ID
    public var title: String
    public var correspondent: String?
    public var documentType: String?
    public var tags: [String]
    public var date: Date
    public var mimeType: String?
    public var pageCount: Int?

    public init(
        id: Document.ID, title: String, correspondent: String? = nil, documentType: String? = nil,
        tags: [String] = [], date: Date, mimeType: String? = nil, pageCount: Int? = nil
    ) {
        self.id = id
        self.title = title
        self.correspondent = correspondent
        self.documentType = documentType
        self.tags = tags
        self.date = date
        self.mimeType = mimeType
        self.pageCount = pageCount
    }

    public init(_ document: Document, correspondent: String?, documentType: String?) {
        self.init(
            id: document.id, title: document.title, correspondent: correspondent,
            documentType: documentType, tags: document.tags.map(\.name), date: document.createdAt,
            mimeType: document.mimeType, pageCount: document.pageCount
        )
    }
}

/// What "Find documents" narrows by. Every field is optional and they combine with AND.
public struct DocumentCriteria: Equatable, Sendable {
    public var tagID: Tag.ID?
    public var correspondentID: Correspondent.ID?
    public var documentTypeID: DocumentType.ID?
    /// First day to include; time of day is ignored.
    public var from: Date?
    /// Last day to include; time of day is ignored.
    public var through: Date?

    public init(
        tagID: Tag.ID? = nil, correspondentID: Correspondent.ID? = nil,
        documentTypeID: DocumentType.ID? = nil, from: Date? = nil, through: Date? = nil
    ) {
        self.tagID = tagID
        self.correspondentID = correspondentID
        self.documentTypeID = documentTypeID
        self.from = from
        self.through = through
    }
}

public enum ShortcutFailure: Error, Equatable, Sendable {
    case notSignedIn
    case noDocuments
    case documentNotFound
    case unreadableFile(String)
}

/// Everything an intent does against the server, kept free of the AppIntents framework so
/// it stays testable and reusable on platforms that get their own intents later.
public struct DocumentCatalog: Sendable {
    private let service: any FolioService
    private let calendar: Calendar
    private let uploads: UploadBatchRunner
    private let uploadScope: String

    /// - Parameters:
    ///   - progress: Where unfinished uploads are remembered, so running the Shortcut again resumes them.
    ///   - uploadScope: Separates remembered uploads by server.
    public init(
        service: any FolioService, calendar: Calendar = .current,
        progress: any UploadProgressStoring = FileUploadProgressStore.shared,
        uploadScope: String = SharedConfiguration.serverURL()?.absoluteString ?? ""
    ) {
        self.service = service
        self.calendar = calendar
        uploads = UploadBatchRunner(service: service, store: progress)
        self.uploadScope = uploadScope
    }

    public func search(_ query: String, limit: Int = 25) async throws -> [DocumentSummary] {
        let documents = try await service.documents(matching: query)
        return try await summaries(Array(documents.prefix(limit)))
    }

    /// Newest first.
    public func recent(limit: Int = 10) async throws -> [DocumentSummary] {
        let documents = try await service.documents(matching: "")
        return try await summaries(Array(Self.newestFirst(documents).prefix(limit)))
    }

    public func latest() async throws -> DocumentSummary {
        guard let first = try await recent(limit: 1).first else { throw ShortcutFailure.noDocuments }
        return first
    }

    public func documents(ids: [Document.ID]) async throws -> [DocumentSummary] {
        let wanted = Set(ids)
        let documents = try await service.documents(matching: "").filter { wanted.contains($0.id) }
        return try await summaries(documents)
    }

    public func find(_ criteria: DocumentCriteria) async throws -> [DocumentSummary] {
        let filter = DocumentFilter(
            tagIDs: criteria.tagID.map { [$0] } ?? [], correspondentID: criteria.correspondentID,
            documentTypeID: criteria.documentTypeID,
            createdAfter: criteria.from.map { calendar.startOfDay(for: $0) },
            createdBefore: criteria.through.flatMap {
                calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: $0))
            })
        return try await summaries(Self.newestFirst(service.documents(filter: filter)))
    }

    public func tags(matching text: String = "") async throws -> [Tag] {
        try await service.tags().filter { Self.matches($0.name, text) }
    }

    public func tags(ids: [Tag.ID]) async throws -> [Tag] {
        try await service.tags().filter { ids.contains($0.id) }
    }

    public func correspondents(matching text: String = "") async throws -> [Correspondent] {
        try await service.correspondents().filter { Self.matches($0.name, text) }
    }

    public func correspondents(ids: [Correspondent.ID]) async throws -> [Correspondent] {
        try await service.correspondents().filter { ids.contains($0.id) }
    }

    public func documentTypes(matching text: String = "") async throws -> [DocumentType] {
        try await service.documentTypes().filter { Self.matches($0.name, text) }
    }

    public func documentTypes(ids: [DocumentType.ID]) async throws -> [DocumentType] {
        try await service.documentTypes().filter { ids.contains($0.id) }
    }

    public func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws {
        try await service.addTag(tagID, to: documentID)
    }

    public func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws {
        try await service.removeTag(tagID, from: documentID)
    }

    /// Downloads the original file.
    public func file(for id: Document.ID) async throws -> (url: URL, document: Document) {
        guard let document = try await service.documents(matching: "").first(where: { $0.id == id })
        else { throw ShortcutFailure.documentNotFound }
        return try await (service.downloadFile(for: document), document)
    }

    /// Uploads each file, then tags every new document. A single file may carry a title.
    ///
    /// When a run stops part-way, running it again with the same files and tags finishes only the remaining
    /// uploads and tags instead of creating the earlier documents again.
    public func upload(
        _ files: [UploadFile], title: String? = nil, tagIDs: [Tag.ID] = []
    ) async throws -> [DocumentSummary] {
        var titled = files
        if files.count == 1, let title = title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            titled[0].title = title
        }
        let uploaded = try await uploads.run(titled, tagIDs: tagIDs, scope: uploadScope)
        return try await summaries(uploaded)
    }

    private func summaries(_ documents: [Document]) async throws -> [DocumentSummary] {
        guard !documents.isEmpty else { return [] }
        async let people = try? service.correspondents()
        async let types = try? service.documentTypes()
        let (loadedPeople, loadedTypes) = await (people, types)
        let peopleByID = Dictionary((loadedPeople ?? []).map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        let typesByID = Dictionary((loadedTypes ?? []).map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        return documents.map { document in
            DocumentSummary(
                document, correspondent: document.correspondentID.flatMap { peopleByID[$0] },
                documentType: document.documentTypeID.flatMap { typesByID[$0] })
        }
    }

    private static func newestFirst(_ documents: [Document]) -> [Document] {
        documents.sorted { $0.createdAt > $1.createdAt }
    }

    private static func matches(_ name: String, _ text: String) -> Bool {
        let needle = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return needle.isEmpty || name.localizedCaseInsensitiveContains(needle)
    }
}
