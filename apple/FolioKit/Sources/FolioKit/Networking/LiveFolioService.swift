import FolioAPI
import Foundation
import OpenAPIRuntime
import OpenAPIURLSession
import UniformTypeIdentifiers

/// Talks to a Folio server. JSON endpoints go through the generated client; file transfer uses
/// URLSession directly because the generated client cannot set per-part content types on uploads
/// or accept the server's file-specific download content types.
public struct LiveFolioService: FolioService {
    private let baseURL: URL
    private let tokenProvider: any BearerTokenProvider
    private let session: URLSession
    private let downloadDirectory: URL?
    private let client: Client
    private let telemetry: FolioTelemetry?
    private let vaultID: UUID?

    public init(
        baseURL: URL,
        tokenProvider: any BearerTokenProvider = NoToken(),
        session: URLSession? = nil,
        telemetry: FolioTelemetry? = nil,
        downloadDirectory: URL? = nil,
        vaultID: UUID? = nil
    ) {
        let session =
            session ?? URLSession(configuration: .ephemeral, delegate: TelemetryRedirectGuard(), delegateQueue: nil)
        self.telemetry = telemetry
        self.baseURL = baseURL
        self.tokenProvider = tokenProvider
        self.session = session
        self.downloadDirectory = downloadDirectory
        self.vaultID = vaultID
        client = Client(
            serverURL: baseURL,
            configuration: .init(dateTranscoder: LenientDateTranscoder()),
            transport: URLSessionTransport(configuration: .init(session: session)),
            // Outermost first: the error mapper sees the response after the bearer middleware retried a 401.
            middlewares: [
                ErrorMappingMiddleware(), VaultScopeMiddleware(vaultID: vaultID),
                BearerAuthMiddleware(provider: tokenProvider),
            ]
        )
    }

    public func scoped(to vaultID: UUID) -> any FolioService {
        LiveFolioService(
            baseURL: baseURL, tokenProvider: tokenProvider, session: session, telemetry: telemetry,
            downloadDirectory: downloadDirectory, vaultID: vaultID
        )
    }

    public func vaults() async throws -> VaultCatalog {
        try await call("vaults.list") {
            let request = URLRequest(url: endpoint("api/vaults"))
            let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
            if (response as? HTTPURLResponse)?.statusCode == 404 {
                return .legacy
            }
            try check(response, body: data)
            guard let catalog = try? JSONDecoder().decode(VaultCatalog.self, from: data),
                let defaultID = catalog.defaultVaultID,
                catalog.vaults.contains(where: { $0.id == defaultID })
            else { throw FolioError.unreadableResponse }
            return catalog
        }
    }

    public func ping() async throws {
        _ = try await tags()
    }

    public func documentPage(query: DocumentBrowseQuery, cursor: String?) async throws -> DocumentPage {
        try await call("documents.browse") {
            let filter = query.filter
            let input = Operations.BrowseDocuments.Input.Query(
                q: query.text.isEmpty ? nil : query.text,
                limit: query.limit, sort: query.sort.rawValue, cursor: cursor,
                correspondentId: filter.correspondentID?.uuidString.lowercased(),
                documentTypeId: filter.documentTypeID?.uuidString.lowercased(),
                tagId: filter.tagIDs.isEmpty ? nil : filter.tagIDs.map { $0.uuidString.lowercased() },
                createdAfter: filter.createdAfter, createdBefore: filter.createdBefore
            )
            let page = try await client.browseDocuments(query: input).ok.body.json
            return try DocumentPage(
                vaultID: Self.uuid(page.vaultId), documents: page.documents.map { try Document($0) },
                nextCursor: page.nextCursor
            )
        }
    }

    public func document(id: Document.ID) async throws -> Document {
        try await call("documents.get") {
            try await Document(client.getDocument(path: .init(id: id.uuidString.lowercased())).ok.body.json)
        }
    }

    public func documents(matching query: String) async throws -> [Document] {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return try await call("documents.list") {
            if trimmed.isEmpty {
                return try await client.listDocuments().ok.body.json.documents.map { try Document($0) }
            }
            let results = try await client.searchDocuments(query: .init(q: trimmed, limit: 50))
            return try results.ok.body.json.results.map { try Document($0.document) }
        }
    }

    public func documents(filter: DocumentFilter) async throws -> [Document] {
        try await call("documents.filter") {
            let query = Operations.ListDocuments.Input.Query(
                correspondentId: filter.correspondentID?.uuidString.lowercased(),
                documentTypeId: filter.documentTypeID?.uuidString.lowercased(),
                tagId: filter.tagIDs.isEmpty ? nil : filter.tagIDs.map { $0.uuidString.lowercased() },
                createdAfter: filter.createdAfter, createdBefore: filter.createdBefore
            )
            return try await client.listDocuments(query: query).ok.body.json.documents.map { try Document($0) }
        }
    }

    /// The server only offers this as a form post that redirects back to the document page.
    public func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws {
        try await call("documents.addTag") { try await addTagRequest(tagID, to: documentID) }
    }

    private func addTagRequest(_ tagID: Tag.ID, to documentID: Document.ID) async throws {
        var request = URLRequest(url: endpoint("api/documents/\(documentID.uuidString.lowercased())/tags"))
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Data("tag_id=\(tagID.uuidString.lowercased())".utf8)
        let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
        try check(response, body: data)
    }

    /// Also a form post, with the tag in the path.
    public func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws {
        try await call("documents.removeTag") { try await removeTagRequest(tagID, from: documentID) }
    }

    private func removeTagRequest(_ tagID: Tag.ID, from documentID: Document.ID) async throws {
        var request = URLRequest(
            url: endpoint(
                "api/documents/\(documentID.uuidString.lowercased())/tags/\(tagID.uuidString.lowercased())"
            )
        )
        request.httpMethod = "POST"
        let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
        try check(response, body: data)
    }

    public func deleteDocument(id: Document.ID) async throws {
        try await call("documents.delete") {
            _ = try await client.deleteDocument(path: .init(id: id.uuidString.lowercased())).noContent
        }
    }

    public func tags() async throws -> [Tag] {
        try await call("tags.list") {
            try await client.listTags().ok.body.json.map {
                try Tag(id: Self.uuid($0.id), name: $0.name, color: $0.color)
            }
        }
    }

    public func correspondents() async throws -> [Correspondent] {
        try await call("correspondents.list") {
            try await client.listCorrespondents().ok.body.json.map {
                try Correspondent(id: Self.uuid($0.id), name: $0.name)
            }
        }
    }

    public func documentTypes() async throws -> [DocumentType] {
        try await call("document_types.list") {
            try await client.listDocumentTypes().ok.body.json.map {
                try DocumentType(id: Self.uuid($0.id), name: $0.name)
            }
        }
    }

    public func downloadFile(for document: Document) async throws -> URL {
        try await call("documents.downloadFile") { try await downloadFileRequest(for: document) }
    }

    private func downloadFileRequest(for document: Document) async throws -> URL {
        let request = URLRequest(url: endpoint("api/documents/\(document.id.uuidString.lowercased())/download"))
        let (temporary, response) = try await authorizedTransfer(request) { try await session.download(for: $0) }
        defer { try? FileManager.default.removeItem(at: temporary) }
        try check(response, body: try? Data(contentsOf: temporary))

        let filename = Self.safeFilename(document.originalFilename, mimeType: document.mimeType)
        let directory =
            downloadDirectory
            ?? FileManager.default.temporaryDirectory
            .appending(path: "folio-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let ext = (filename as NSString).pathExtension
        let suffix = !ext.isEmpty && ext.utf8.count <= 32 ? "." + ext : ""
        let destination = directory.appending(
            path: downloadDirectory == nil ? filename : UUID().uuidString + suffix
        )
        do {
            try FileManager.default.moveItem(at: temporary, to: destination)
        } catch {
            if downloadDirectory == nil {
                try? FileManager.default.removeItem(at: directory)
            }
            throw error
        }
        return destination
    }

    public func upload(_ file: UploadFile) async throws -> Document {
        try await call("documents.upload") { try await uploadRequest(file) }
    }

    private func uploadRequest(_ file: UploadFile) async throws -> Document {
        var request = URLRequest(url: endpoint("api/documents/upload"))
        request.httpMethod = "POST"
        let form = MultipartForm(file: file)
        request.setValue(form.contentType, forHTTPHeaderField: "Content-Type")

        let (data, response) = try await authorizedTransfer(request) {
            try await session.upload(for: $0, from: form.body)
        }
        try check(response, body: data)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let string = try decoder.singleValueContainer().decode(String.self)
            return try LenientDateTranscoder().decode(string)
        }
        do {
            return try Document(decoder.decode(Components.Schemas.Document.self, from: data))
        } catch {
            throw FolioError.unreadableResponse
        }
    }

    /// The server controls the filename, so keep only its last path component and never a traversal.
    static func safeFilename(_ name: String?, mimeType: String?) -> String {
        let last = name.map { ($0 as NSString).lastPathComponent } ?? ""
        if !["", ".", "..", "/"].contains(last) {
            return last
        }
        let ext = mimeType.flatMap { UTType(mimeType: $0)?.preferredFilenameExtension }
        return ext.map { "document.\($0)" } ?? "document"
    }

    fileprivate static func uuid(_ string: String) throws -> UUID {
        guard let id = UUID(uuidString: string) else { throw FolioError.unreadableResponse }
        return id
    }

    private func endpoint(_ path: String) -> URL {
        baseURL.appending(path: path)
    }

    /// Sends a request with the bearer token; if the server rejects it, refreshes the token and sends it once more.
    private func authorizedTransfer<T>(
        _ request: URLRequest, _ perform: (URLRequest) async throws -> (T, URLResponse)
    ) async throws -> (T, URLResponse) {
        func signed(_ token: String?) -> URLRequest {
            var copy = request
            if let vaultID {
                copy.setValue(vaultID.uuidString.lowercased(), forHTTPHeaderField: "X-Folio-Vault")
            }
            if let token {
                copy.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            }
            return copy
        }
        let token = await tokenProvider.token()
        let first = try await transfer { try await perform(signed(token)) }
        guard (first.1 as? HTTPURLResponse)?.statusCode == 401, let token,
            let fresh = await tokenProvider.refresh(rejecting: token)
        else { return first }
        return try await transfer { try await perform(signed(fresh)) }
    }

    private func transfer<T>(_ work: () async throws -> T) async throws -> T {
        do {
            return try await work()
        } catch is URLError {
            throw FolioError.unreachable
        }
    }

    private func check(_ response: URLResponse, body: Data?) throws {
        guard let http = response as? HTTPURLResponse else { throw FolioError.unreadableResponse }
        if http.statusCode == 401 {
            throw FolioError.signInRequired
        }
        guard (200..<300).contains(http.statusCode) else {
            throw FolioError.server(status: http.statusCode, message: body.flatMap(ErrorMappingMiddleware.message))
        }
    }

    private func call<T>(_ name: String, _ work: () async throws -> T) async throws -> T {
        if let telemetry {
            return try await telemetry.operation(name) { try await mappedCall(work) }
        }
        return try await mappedCall(work)
    }

    private func mappedCall<T>(_ work: () async throws -> T) async throws -> T {
        do {
            return try await work()
        } catch let error as FolioError {
            throw error
        } catch let error as ClientError {
            switch error.underlyingError {
            case let folio as FolioError: throw folio
            case is URLError: throw FolioError.unreachable
            default: throw FolioError.unreadableResponse
            }
        }
    }
}

extension Document {
    init(_ raw: Components.Schemas.Document) throws {
        try self.init(
            id: LiveFolioService.uuid(raw.id), title: raw.title, content: raw.content,
            originalFilename: raw.originalFilename, mimeType: raw.mimeType,
            correspondentID: raw.correspondentId.map(LiveFolioService.uuid),
            documentTypeID: raw.documentTypeId.map(LiveFolioService.uuid),
            pageCount: raw.pageCount.map(Int.init),
            ocrStatus: .init(raw.ocrStatus), embedStatus: .init(raw.embedStatus),
            createdAt: raw.createdAt, updatedAt: raw.updatedAt
        )
    }
}

extension Document.ProcessingStatus {
    init(_ raw: Components.Schemas.ProcessingStatus) {
        switch raw {
        case .pending: self = .pending
        case .running: self = .running
        case .done: self = .done
        case .failed: self = .failed
        case .skipped: self = .skipped
        }
    }
}

extension Document {
    /// The list and get endpoints add the document's tags to the plain document.
    init(_ raw: Components.Schemas.DocumentView) throws {
        try self.init(raw.value1)
        tags = try raw.value2.tags.map {
            try Tag(id: LiveFolioService.uuid($0.id), name: $0.name, color: $0.color)
        }
    }
}
