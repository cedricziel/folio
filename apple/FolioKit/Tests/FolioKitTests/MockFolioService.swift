import Foundation

@testable import FolioKit

actor MockFolioService: FolioService {
    private var savedViewValues: [SavedView] = []
    private var savedViewsGate: CheckedContinuation<Void, Never>?
    private var savedViewsStarted: CheckedContinuation<Void, Never>?
    private var suspendSavedViewRequests = false

    func suspendSavedViews() { suspendSavedViewRequests = true }
    func waitForSavedViewsRequest() async {
        guard savedViewsGate == nil else { return }
        await withCheckedContinuation { savedViewsStarted = $0 }
    }
    func releaseSavedViews() {
        suspendSavedViewRequests = false
        savedViewsGate?.resume()
        savedViewsGate = nil
    }
    func setSavedViews(_ views: [SavedView]) { savedViewValues = views }

    private var detailGate: CheckedContinuation<Void, Never>?
    private var detailStarted: CheckedContinuation<Void, Never>?
    private var suspendDetailRequests = false

    func suspendDocumentReads() { suspendDetailRequests = true }
    func waitForDocumentRead() async {
        guard detailGate == nil else { return }
        await withCheckedContinuation { detailStarted = $0 }
    }
    func releaseDocumentReads() {
        suspendDetailRequests = false
        detailGate?.resume()
        detailGate = nil
    }

    func savedViews(after: UUID?) async throws -> [SavedView] {
        if let failure { throw failure }
        let values = Array(
            savedViewValues.sorted { $0.id.uuidString < $1.id.uuidString }
                .filter { view in after.map { view.id.uuidString > $0.uuidString } ?? true }.prefix(50))
        if suspendSavedViewRequests {
            await withCheckedContinuation { continuation in
                savedViewsGate = continuation
                savedViewsStarted?.resume()
                savedViewsStarted = nil
            }
        }
        return values
    }

    func savedView(id: UUID) async throws -> SavedView {
        guard let view = savedViewValues.first(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: "Saved view not found")
        }
        return view
    }

    func saveView(_ draft: SavedViewDraft) async throws -> SavedView {
        if let failure { throw failure }
        let view = SavedView(
            id: UUID(), vaultID: vaultCatalog.defaultVaultID ?? Self.defaultVault.id,
            ownerID: "test-user", name: draft.name, visibility: draft.visibility, query: draft.query,
            presentation: draft.presentation, createdAt: .distantPast, updatedAt: .distantPast,
            createdBy: "test-user", updatedBy: "test-user")
        savedViewValues.append(view)
        return view
    }

    func updateSavedView(id: UUID, draft: SavedViewDraft) async throws -> SavedView {
        if let failure { throw failure }
        let original = try await savedView(id: id)
        let view = SavedView(
            id: id, vaultID: original.vaultID, ownerID: original.ownerID,
            name: draft.name, visibility: draft.visibility,
            query: draft.replacesQuery ? draft.query : original.query, presentation: draft.presentation,
            createdAt: original.createdAt, updatedAt: original.updatedAt.addingTimeInterval(1),
            createdBy: original.createdBy, updatedBy: "test-user")
        savedViewValues.removeAll { $0.id == id }
        savedViewValues.append(view)
        return view
    }

    func deleteSavedView(id: UUID) async throws {
        savedViewValues.removeAll { $0.id == id }
    }

    static let defaultVault = Vault(
        id: UUID(uuidString: "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff")!, name: "Personal", role: .owner
    )
    static let defaultCatalog = VaultCatalog(vaults: [defaultVault], defaultVaultID: defaultVault.id)
    private var entries: [DocumentEntry] = []
    private var entryReceipts: [UUID: DocumentEntry] = [:]
    private var loseFirstEntryResponse: Bool
    private var vaultCatalog: VaultCatalog
    nonisolated let vaultServices: [UUID: any FolioService]
    var documentList: [Document] = []
    var correspondentList: [Correspondent] = []
    var typeList: [DocumentType] = []
    var failure: FolioError?
    var failUploadsAfter: Int?
    /// Creates the document but loses the answer, as when the connection drops after the server stored it.
    var loseUploadResponses = false
    var failingTagIDs: Set<Tag.ID> = []
    /// Documents someone deleted; tagging them answers 404.
    var deletedDocumentIDs: Set<Document.ID> = []
    private var keyedUploads: [UUID: Document] = [:]
    private(set) var uploadKeys: [UUID] = []
    var failLookups = false
    var uploadDelay: Duration?
    var tagList: [Tag] = []
    private(set) var tagged: [TagAssignment] = []
    private(set) var untagged: [TagAssignment] = []
    private(set) var queries: [String] = []
    private(set) var uploads: [UploadFile] = []
    private(set) var deleted: [Document.ID] = []
    private var trashedDocuments: [Document.ID: Document] = [:]
    private(set) var pings = 0
    private var documentGate: CheckedContinuation<[Document], Never>?
    private var documentStarted: CheckedContinuation<Void, Never>?
    private var suspendDocumentRequests = false
    private var uploadGate: CheckedContinuation<Void, Never>?
    private var uploadStarted: CheckedContinuation<Void, Never>?
    private var suspendUploadRequests = false

    init(
        documents: [Document] = [], correspondents: [Correspondent] = [], types: [DocumentType] = [],
        vaults: VaultCatalog = MockFolioService.defaultCatalog, vaultServices: [UUID: any FolioService] = [:],
        loseFirstEntryResponse: Bool = false
    ) {
        self.loseFirstEntryResponse = loseFirstEntryResponse
        vaultCatalog = vaults
        self.vaultServices = vaultServices
        documentList = documents
        correspondentList = correspondents
        typeList = types
    }

    func entryPage(documentID: UUID, kind: DocumentEntryKind, cursor: String?) throws -> DocumentEntryPage {
        if let failure { throw failure }
        return DocumentEntryPage(
            entries: entries.filter { $0.documentID == documentID && $0.kind == kind }, nextCursor: nil, canCreate: true
        )
    }

    func createEntry(documentID: UUID, kind: DocumentEntryKind, body: String, key: UUID) throws -> DocumentEntry {
        if let failure { throw failure }
        if let receipt = entryReceipts[key] { return receipt }
        let now = Date()
        let entry = DocumentEntry(
            id: UUID(), documentID: documentID, vaultID: Self.defaultVault.id,
            kind: kind, body: body, createdAt: now, updatedAt: now, createdBy: "Fixture author",
            canEdit: true, canDelete: true)
        entries.append(entry)
        entryReceipts[key] = entry
        if loseFirstEntryResponse {
            loseFirstEntryResponse = false
            throw FolioError.unreachable
        }
        return entry
    }

    func updateEntry(documentID: UUID, entryID: UUID, body: String) throws -> DocumentEntry {
        if let failure { throw failure }
        guard let index = entries.firstIndex(where: { $0.id == entryID && $0.documentID == documentID }) else {
            throw FolioError.server(status: 404, message: nil)
        }
        let original = entries[index]
        let entry = DocumentEntry(
            id: original.id, documentID: documentID, vaultID: original.vaultID,
            kind: original.kind, body: body, createdAt: original.createdAt, updatedAt: Date(),
            createdBy: original.createdBy, canEdit: true, canDelete: true)
        entries[index] = entry
        return entry
    }

    func deleteEntry(documentID: UUID, entryID: UUID) throws {
        if let failure { throw failure }
        entries.removeAll { $0.id == entryID && $0.documentID == documentID }
    }

    func vaults() async throws -> VaultCatalog {
        if let failure {
            throw failure
        }
        return vaultCatalog
    }

    func setVaults(_ catalog: VaultCatalog) {
        vaultCatalog = catalog
    }

    nonisolated func scoped(to vaultID: UUID) -> any FolioService {
        vaultServices[vaultID] ?? self
    }

    func suspendDocuments() {
        suspendDocumentRequests = true
    }

    func waitForDocumentRequest() async {
        guard documentGate == nil else { return }
        await withCheckedContinuation { documentStarted = $0 }
    }

    func releaseDocuments() {
        suspendDocumentRequests = false
        documentGate?.resume(returning: documentList)
        documentGate = nil
    }

    func suspendUploads() {
        suspendUploadRequests = true
    }

    func waitForUploadRequest() async {
        guard uploadGate == nil else { return }
        await withCheckedContinuation { uploadStarted = $0 }
    }

    func releaseUploads() {
        suspendUploadRequests = false
        uploadGate?.resume()
        uploadGate = nil
    }

    func setDocuments(_ documents: [Document]) {
        documentList = documents
    }

    func setFailure(_ error: FolioError?) {
        failure = error
    }

    func setFailUploadsAfter(_ count: Int?) {
        failUploadsAfter = count
    }

    func setLoseUploadResponses(_ lose: Bool) {
        loseUploadResponses = lose
    }

    func setFailingTags(_ ids: Set<Tag.ID>) {
        failingTagIDs = ids
    }

    func markDeleted(_ id: Document.ID) {
        deletedDocumentIDs.insert(id)
    }

    func setTags(_ tags: [Tag]) {
        tagList = tags
    }

    func setUploadDelay(_ delay: Duration?) {
        uploadDelay = delay
    }

    func setFailLookups(_ value: Bool) {
        failLookups = value
    }

    func ping() async throws {
        pings += 1
        if let failure {
            throw failure
        }
    }

    private let pageVaultID = UUID()
    private var pages: [String: DocumentPage] = [:]
    private(set) var pageRequests: [String] = []
    private var suspendedCursor: String?
    private var pageGate: CheckedContinuation<DocumentPage, Never>?
    private var pageStarted: CheckedContinuation<Void, Never>?

    func setPages(_ pages: [String: DocumentPage]) { self.pages = pages }
    func suspendPage(_ cursor: String) { suspendedCursor = cursor }
    func waitForPage() async {
        guard pageGate == nil else { return }
        await withCheckedContinuation { pageStarted = $0 }
    }
    func releasePage(_ page: DocumentPage) {
        suspendedCursor = nil
        pageGate?.resume(returning: page)
        pageGate = nil
    }

    func documentPage(query: DocumentBrowseQuery, cursor: String?) async throws -> DocumentPage {
        let key = query.text + "|" + (cursor ?? "")
        pageRequests.append(key)
        if let cursor, cursor == suspendedCursor {
            return await withCheckedContinuation {
                pageGate = $0
                pageStarted?.resume()
                pageStarted = nil
            }
        }
        if let failure { throw failure }
        if let page = pages[key] { return page }
        return try await DocumentPage(vaultID: pageVaultID, documents: documents(matching: query.text))
    }

    func setDocumentReview(_ reviewed: Bool, for id: Document.ID) async throws -> Document {
        if let failure { throw failure }
        guard let index = documentList.firstIndex(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: nil)
        }
        documentList[index].reviewed = reviewed
        return documentList[index]
    }

    func setDocumentDate(_ date: DocumentDate?, for id: Document.ID) async throws -> Document {
        if let failure { throw failure }
        guard let index = documentList.firstIndex(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: nil)
        }
        documentList[index].documentDate = date
        return documentList[index]
    }

    func document(id: Document.ID) async throws -> Document {
        if let failure { throw failure }
        guard let document = documentList.first(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: nil)
        }
        if suspendDetailRequests {
            await withCheckedContinuation { continuation in
                detailGate = continuation
                detailStarted?.resume()
                detailStarted = nil
            }
        }
        return document
    }

    func documents(matching query: String) async throws -> [Document] {
        queries.append(query)
        if let failure {
            throw failure
        }
        if suspendDocumentRequests {
            return await withCheckedContinuation {
                documentGate = $0
                documentStarted?.resume()
                documentStarted = nil
            }
        }
        return documentList
    }

    func deleteDocument(id: Document.ID) async throws {
        if let failure {
            throw failure
        }
        deleted.append(id)
        trashedDocuments[id] = documentList.first { $0.id == id }
        documentList.removeAll { $0.id == id }
    }

    func restoreDocument(id: Document.ID) async throws -> Document {
        if let failure {
            throw failure
        }
        guard let document = trashedDocuments.removeValue(forKey: id) else {
            throw FolioError.server(status: 404, message: nil)
        }
        documentList.append(document)
        return document
    }

    private var thumbnailData: Data?
    private var thumbnailGates: [UUID: CheckedContinuation<Void, any Error>] = [:]
    private var thumbnailStarted: CheckedContinuation<Void, Never>?
    private var suspendThumbnailRequests = false
    private(set) var cancelledThumbnailRequests = 0

    func suspendThumbnails() { suspendThumbnailRequests = true }

    func waitForThumbnailRequest() async {
        guard thumbnailGates.isEmpty else { return }
        await withCheckedContinuation { thumbnailStarted = $0 }
    }

    func releaseThumbnails() {
        suspendThumbnailRequests = false
        let gates = thumbnailGates
        thumbnailGates.removeAll()
        for gate in gates.values { gate.resume() }
    }

    private func cancelThumbnail(_ request: UUID) {
        guard let gate = thumbnailGates.removeValue(forKey: request) else { return }
        cancelledThumbnailRequests += 1
        gate.resume(throwing: CancellationError())
    }

    private var thumbnailOutcomes: [Document.ID: Result<Data?, FolioError>] = [:]
    private(set) var thumbnailRequests: [Document.ID] = []

    func setThumbnail(_ data: Data?) { thumbnailData = data }

    func setThumbnail(_ data: Data?, for id: Document.ID, error: FolioError? = nil) {
        thumbnailOutcomes[id] = error.map { .failure($0) } ?? .success(data)
    }

    func thumbnail(id: Document.ID) async throws -> Data? {
        try Task.checkCancellation()
        thumbnailRequests.append(id)
        if suspendThumbnailRequests {
            let request = UUID()
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { (gate: CheckedContinuation<Void, any Error>) in
                    thumbnailGates[request] = gate
                    thumbnailStarted?.resume()
                    thumbnailStarted = nil
                }
            } onCancel: {
                Task { await self.cancelThumbnail(request) }
            }
        }
        try Task.checkCancellation()
        if let result = thumbnailOutcomes[id] { return try result.get() }
        if let failure { throw failure }
        return thumbnailData
    }

    func downloadFile(for document: Document) async throws -> URL {
        try Task.checkCancellation()
        if let failure {
            throw failure
        }
        return URL(filePath: "/tmp/\(document.originalFilename ?? "file")")
    }

    func upload(_ file: UploadFile) async throws -> Document {
        try await store(file, idempotencyKey: nil)
    }

    /// Like the server: a key seen before returns its document, or 410 once that document was deleted.
    func upload(_ file: UploadFile, idempotencyKey: UUID) async throws -> Document {
        uploadKeys.append(idempotencyKey)
        if let earlier = keyedUploads[idempotencyKey] {
            if deletedDocumentIDs.contains(earlier.id) {
                throw FolioError.server(status: 410, message: nil)
            }
            return earlier
        }
        return try await store(file, idempotencyKey: idempotencyKey)
    }

    private func store(_ file: UploadFile, idempotencyKey: UUID?) async throws -> Document {
        if suspendUploadRequests {
            await withCheckedContinuation {
                uploadGate = $0
                uploadStarted?.resume()
                uploadStarted = nil
            }
        }
        if let uploadDelay {
            try await Task.sleep(for: uploadDelay)
        }
        if let failure {
            throw failure
        }
        if let limit = failUploadsAfter, uploads.count >= limit {
            throw FolioError.unreachable
        }
        uploads.append(file)
        let document = Document(title: file.title ?? file.filename, originalFilename: file.filename)
        if let idempotencyKey {
            keyedUploads[idempotencyKey] = document
        }
        if loseUploadResponses {
            documentList.append(document)
            throw FolioError.unreachable
        }
        return document
    }

    func tags() async throws -> [Tag] {
        tagList
    }

    private(set) var filters: [DocumentFilter] = []

    func documents(filter: DocumentFilter) async throws -> [Document] {
        filters.append(filter)
        if let failure {
            throw failure
        }
        return documentList.filter { document in
            (filter.correspondentID == nil || document.correspondentID == filter.correspondentID)
                && (filter.documentTypeID == nil || document.documentTypeID == filter.documentTypeID)
                && filter.tagIDs.allSatisfy { id in document.tags.contains { $0.id == id } }
                && (filter.createdAfter.map { document.createdAt >= $0 } ?? true)
                && (filter.createdBefore.map { document.createdAt < $0 } ?? true)
        }
    }

    func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws {
        if let failure {
            throw failure
        }
        if deletedDocumentIDs.contains(documentID) {
            throw FolioError.server(status: 404, message: nil)
        }
        if failingTagIDs.contains(tagID) {
            throw FolioError.server(status: 500, message: nil)
        }
        tagged.append(TagAssignment(tag: tagID, document: documentID))
    }

    func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws {
        if let failure {
            throw failure
        }
        untagged.append(TagAssignment(tag: tagID, document: documentID))
    }

    func correspondents() async throws -> [Correspondent] {
        if failLookups {
            throw FolioError.unreachable
        }
        return correspondentList
    }

    var trashedTaxonomy: [TaxonomyKind: [TaxonomyEntry]] = [:]

    func setTrashedTaxonomy(_ entries: [TaxonomyEntry], kind: TaxonomyKind) {
        trashedTaxonomy[kind] = entries
    }

    func setCorrespondents(_ correspondents: [Correspondent]) {
        correspondentList = correspondents
    }

    func taxonomy(_ kind: TaxonomyKind, deleted: Bool) async throws -> [TaxonomyEntry] {
        deleted ? trashedTaxonomy[kind, default: []] : []
    }

    func documentTypes() async throws -> [DocumentType] {
        if failLookups {
            throw FolioError.unreachable
        }
        return typeList
    }
}

final class MemoryServerConfigStore: ServerConfigStore, @unchecked Sendable {
    var url: URL?
    init(url: URL? = nil) {
        self.url = url
    }

    func load() -> URL? {
        url
    }

    func save(_ url: URL?) {
        self.url = url
    }
}

struct TagAssignment: Equatable {
    let tag: Tag.ID
    let document: Document.ID
}
