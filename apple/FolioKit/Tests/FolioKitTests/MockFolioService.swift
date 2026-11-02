import Foundation

@testable import FolioKit

actor MockFolioService: FolioService {
    static let defaultVault = Vault(
        id: UUID(uuidString: "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff")!, name: "Personal", role: .owner
    )
    static let defaultCatalog = VaultCatalog(vaults: [defaultVault], defaultVaultID: defaultVault.id)
    private var vaultCatalog: VaultCatalog
    nonisolated let vaultServices: [UUID: any FolioService]
    var documentList: [Document] = []
    var correspondentList: [Correspondent] = []
    var typeList: [DocumentType] = []
    var failure: FolioError?
    var failUploadsAfter: Int?
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
        vaults: VaultCatalog = MockFolioService.defaultCatalog, vaultServices: [UUID: any FolioService] = [:]
    ) {
        vaultCatalog = vaults
        self.vaultServices = vaultServices
        documentList = documents
        correspondentList = correspondents
        typeList = types
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

    func document(id: Document.ID) async throws -> Document {
        if let failure { throw failure }
        guard let document = documentList.first(where: { $0.id == id }) else {
            throw FolioError.server(status: 404, message: nil)
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

    func thumbnail(id _: Document.ID) async throws -> Data? {
        if let failure { throw failure }
        return nil
    }

    func downloadFile(for document: Document) async throws -> URL {
        try Task.checkCancellation()
        if let failure {
            throw failure
        }
        return URL(filePath: "/tmp/\(document.originalFilename ?? "file")")
    }

    func upload(_ file: UploadFile) async throws -> Document {
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
        return Document(title: file.title ?? file.filename, originalFilename: file.filename)
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
