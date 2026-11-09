import Foundation
import UniformTypeIdentifiers
import os

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
    public var originalFilename: String?
    /// Whether the document's text is known to be non-empty, so it can be shared as plain text.
    public var hasRecognizedText: Bool
    /// The vault holding the document; nil is the server's default vault.
    public var vaultID: UUID?
    /// Whether others can reach the document's vault.
    public var vaultSharing: VaultSharing

    public init(
        id: Document.ID, title: String, correspondent: String? = nil, documentType: String? = nil,
        tags: [String] = [], date: Date, mimeType: String? = nil, pageCount: Int? = nil,
        originalFilename: String? = nil, hasRecognizedText: Bool = false, vaultID: UUID? = nil,
        vaultSharing: VaultSharing = .unknown
    ) {
        self.id = id
        self.title = title
        self.correspondent = correspondent
        self.documentType = documentType
        self.tags = tags
        self.date = date
        self.mimeType = mimeType
        self.pageCount = pageCount
        self.originalFilename = originalFilename
        self.hasRecognizedText = hasRecognizedText
        self.vaultID = vaultID
        self.vaultSharing = vaultSharing
    }

    public init(
        _ document: Document, correspondent: String?, documentType: String?, vaultID: UUID? = nil,
        vaultSharing: VaultSharing = .unknown
    ) {
        self.init(
            id: document.id, title: document.title, correspondent: correspondent,
            documentType: documentType, tags: document.tags.map(\.name), date: document.createdAt,
            mimeType: document.mimeType, pageCount: document.pageCount,
            originalFilename: document.originalFilename,
            hasRecognizedText: !(document.content ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
            vaultID: vaultID, vaultSharing: vaultSharing
        )
    }

    /// The fields Spotlight shows, so processing progress alone doesn't re-index a document.
    public var indexedFields: [String] {
        [title, correspondent ?? "", documentType ?? "", FolioFormat.isoDay(date)] + tags
    }
}

/// The outcome of looking documents up by id. Documents that are gone or no longer readable are missing;
/// those the server could not answer for right now are unresolved.
public struct DocumentResolution: Sendable {
    public var found: [DocumentSummary]
    public var missing: [DocumentEntityID]
    public var unresolved: [DocumentEntityID]
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
    case noRecognizedText
    case unreadableFile(String)
    /// The items an intent was given live in different vaults.
    case mixedVaults
}

/// A document's original file, downloaded for sharing.
public struct DocumentTransferFile: Sendable {
    public let url: URL
    public let contentType: UTType
}

/// Everything an intent does against the server, kept free of the AppIntents framework so
/// it stays testable and reusable on platforms that get their own intents later.
public struct DocumentCatalog: Sendable {
    let service: any FolioService
    private let calendar: Calendar
    private let progress: any UploadProgressStoring
    private let uploads: UploadBatchRunner
    private let uploadScope: String
    /// The vault `service` is scoped to; nil is the server's default vault.
    private let vaultID: UUID?

    /// - Parameters:
    ///   - progress: Where unfinished uploads are remembered, so running the Shortcut again resumes them.
    ///   - uploadScope: Separates remembered uploads by server.
    public init(
        service: any FolioService, calendar: Calendar = .current,
        progress: any UploadProgressStoring = FileUploadProgressStore.shared,
        uploadScope: String = SharedConfiguration.serverURL()?.absoluteString ?? "",
        vaultID: UUID? = nil
    ) {
        self.service = service
        self.calendar = calendar
        self.progress = progress
        uploads = UploadBatchRunner(service: service, store: progress)
        self.uploadScope = uploadScope
        self.vaultID = vaultID
    }

    /// The same catalog in another vault; nil keeps this one.
    public func scoped(to vaultID: UUID?) -> DocumentCatalog {
        guard let vaultID, vaultID != self.vaultID else { return self }
        return DocumentCatalog(
            service: service.scoped(to: vaultID), calendar: calendar, progress: progress, uploadScope: uploadScope,
            vaultID: vaultID)
    }

    public func vaults() async throws -> VaultCatalog {
        try await service.vaults()
    }

    /// The vault an intent works in: the one asked for, else the one its tags, correspondent or type come from.
    /// Items without a vault fit any vault.
    public static func vault(asked: UUID?, implied: [UUID?]) throws -> UUID? {
        let vaults = Set(([asked] + implied).compactMap { $0 })
        guard vaults.count <= 1 else { throw ShortcutFailure.mixedVaults }
        return vaults.first
    }

    /// Runs `load` in every vault the account can reach. Without a vault list it runs here only. A vault that fails
    /// is left out, so one unreadable vault doesn't empty a picker; it throws only when every vault failed.
    public func acrossVaults<Value: Sendable>(
        _ load: (DocumentCatalog) async throws -> [Value]
    ) async throws -> [VaultScoped<Value>] {
        let vaults = (try? await service.vaults().vaults) ?? []
        guard !vaults.isEmpty else { return try await load(self).map { VaultScoped($0, vaultID: vaultID) } }
        var found: [VaultScoped<Value>] = []
        var firstError: (any Error)?
        var loaded = 0
        for vault in vaults {
            let name = vaults.count > 1 ? vault.name : nil
            do {
                found += try await load(scoped(to: vault.id)).map {
                    VaultScoped($0, vaultID: vault.id, vaultName: name)
                }
                loaded += 1
            } catch {
                try Task.checkCancellation()
                Self.logger.error("Skipped a vault that failed to load: \(String(describing: error), privacy: .public)")
                firstError = firstError ?? error
            }
        }
        if loaded == 0, let firstError { throw firstError }
        return found
    }

    /// Looks items up in the vault each id names, keeping the order vaults first appear in.
    public func lookup<Value: Sendable>(
        _ ids: [VaultScopedID], _ load: (DocumentCatalog, [UUID]) async throws -> [Value]
    ) async throws -> [VaultScoped<Value>] {
        var order: [UUID?] = []
        var byVault: [UUID?: [UUID]] = [:]
        for id in ids {
            if byVault[id.vaultID] == nil { order.append(id.vaultID) }
            byVault[id.vaultID, default: []].append(id.id)
        }
        var found: [VaultScoped<Value>] = []
        for vault in order {
            found += try await load(scoped(to: vault), byVault[vault] ?? []).map { VaultScoped($0, vaultID: vault) }
        }
        return found
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

    public func documents(ids: [DocumentEntityID]) async throws -> [DocumentSummary] {
        try await resolve(ids: ids).found
    }

    /// Looks each document up by id in its own vault, a few at a time, in the order asked.
    /// Throws only when the server answered for none of them.
    public func resolve(ids: [DocumentEntityID], concurrency: Int = 4) async throws -> DocumentResolution {
        var unique: [DocumentEntityID] = []
        var seen = Set<DocumentEntityID>()
        for id in ids where seen.insert(id).inserted { unique.append(id) }

        var outcomes = [Result<Document?, any Error>?](repeating: nil, count: unique.count)
        await withTaskGroup(of: (Int, Result<Document?, any Error>).self) { group in
            var next = 0
            func start() {
                guard next < unique.count else { return }
                let position = next
                let catalog = scoped(to: unique[position].vaultID)
                let documentID = unique[position].documentID
                group.addTask {
                    do {
                        return (position, .success(try await catalog.document(documentID)))
                    } catch ShortcutFailure.documentNotFound {
                        return (position, .success(nil))
                    } catch {
                        return (position, .failure(error))
                    }
                }
                next += 1
            }
            for _ in 0..<max(1, concurrency) { start() }
            for await (position, outcome) in group {
                outcomes[position] = outcome
                start()
            }
        }

        var resolution = DocumentResolution(found: [], missing: [], unresolved: [])
        var firstError: (any Error)?
        var foundByVault: [UUID?: [Document]] = [:]
        var vaultOrder: [UUID?] = []
        for (id, outcome) in zip(unique, outcomes) {
            switch outcome {
            case let .success(document?):
                if foundByVault[id.vaultID] == nil { vaultOrder.append(id.vaultID) }
                foundByVault[id.vaultID, default: []].append(document)
            case .success(nil):
                resolution.missing.append(id)
            case let .failure(error):
                firstError = firstError ?? error
                resolution.unresolved.append(id)
            case nil:
                resolution.unresolved.append(id)
            }
        }
        if let firstError, foundByVault.isEmpty, resolution.missing.isEmpty { throw firstError }
        var summariesByID: [DocumentEntityID: DocumentSummary] = [:]
        for vault in vaultOrder {
            for summary in try await scoped(to: vault).summaries(foundByVault[vault] ?? []) {
                summariesByID[DocumentEntityID(documentID: summary.id, vaultID: vault)] = summary
            }
        }
        resolution.found = unique.compactMap { summariesByID[$0] }
        return resolution
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

    /// Downloads the original file through the authenticated download path, in the document's vault.
    public func file(for id: DocumentEntityID) async throws -> (url: URL, document: Document) {
        let catalog = scoped(to: id.vaultID)
        let document = try await catalog.document(id.documentID)
        return try await (catalog.service.downloadFile(for: document), document)
    }

    /// The original file, named as it was uploaded, with the type its content declares. A copy already on
    /// this device, such as the open preview, is reused; a new download is kept in `cache`, which removes it.
    public func transferFile(
        for id: DocumentEntityID, cache: TransferFileCache = .shared
    ) async throws -> DocumentTransferFile {
        if let cached = cache.file(for: id) {
            return DocumentTransferFile(
                url: cached,
                contentType: DocumentTransfer.contentType(mimeType: nil, filename: cached.lastPathComponent))
        }
        let (download, document) = try await file(for: id)
        let url = try cache.store(download, for: id)
        return DocumentTransferFile(
            url: url,
            contentType: DocumentTransfer.contentType(mimeType: document.mimeType, filename: document.originalFilename)
        )
    }

    /// The text the server read from the document.
    public func recognizedText(for id: DocumentEntityID) async throws -> String {
        let document = try await scoped(to: id.vaultID).document(id.documentID)
        let text = document.content?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !text.isEmpty else { throw ShortcutFailure.noRecognizedText }
        return text
    }

    /// A readable, untrashed document; anything the server won't show this account counts as not found.
    private func document(_ id: Document.ID) async throws -> Document {
        let document: Document
        do {
            document = try await service.document(id: id)
        } catch FolioError.server(status: 404, _), FolioError.server(status: 403, _) {
            throw ShortcutFailure.documentNotFound
        }
        guard document.id == id, document.deletedAt == nil else { throw ShortcutFailure.documentNotFound }
        return document
    }

    /// Uploads each file, then tags every new document. A single file may carry a title.
    ///
    /// When a run stops part-way, running it again with the same files and tags finishes only the remaining
    /// uploads and tags instead of creating the earlier documents again.
    public func upload(
        _ files: [UploadFile], title: String? = nil, tagIDs: [Tag.ID] = [],
        finished: @Sendable (Int) -> Void = { _ in }
    ) async throws -> [DocumentSummary] {
        var titled = files
        if files.count == 1, let title = title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            titled[0].title = title
        }
        let uploaded = try await uploads.run(titled, tagIDs: tagIDs, scope: uploadScope, finished: finished)
        return try await summaries(uploaded)
    }

    private func summaries(_ documents: [Document]) async throws -> [DocumentSummary] {
        guard !documents.isEmpty else { return [] }
        async let people = try? service.correspondents()
        async let types = try? service.documentTypes()
        async let vaults = try? service.vaults()
        let (loadedPeople, loadedTypes, loadedVaults) = await (people, types, vaults)
        let vault = loadedVaults.flatMap { catalog in
            catalog.vaults.first { $0.id == (vaultID ?? catalog.defaultVaultID) }
        }
        let sharing = vault?.sharing ?? .unknown
        let peopleByID = Dictionary((loadedPeople ?? []).map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        let typesByID = Dictionary((loadedTypes ?? []).map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        return documents.map { document in
            DocumentSummary(
                document, correspondent: document.correspondentID.flatMap { peopleByID[$0] },
                documentType: document.documentTypeID.flatMap { typesByID[$0] }, vaultID: vaultID,
                vaultSharing: sharing)
        }
    }

    private static let logger = Logger(subsystem: "com.cedricziel.folio", category: "shortcuts")

    private static func newestFirst(_ documents: [Document]) -> [Document] {
        documents.sorted { $0.createdAt > $1.createdAt }
    }

    private static func matches(_ name: String, _ text: String) -> Bool {
        let needle = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return needle.isEmpty || name.localizedCaseInsensitiveContains(needle)
    }
}
