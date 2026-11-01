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
    private let client: Client

    public init(
        baseURL: URL,
        tokenProvider: any BearerTokenProvider = NoToken(),
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.tokenProvider = tokenProvider
        self.session = session
        client = Client(
            serverURL: baseURL,
            configuration: .init(dateTranscoder: LenientDateTranscoder()),
            transport: URLSessionTransport(configuration: .init(session: session)),
            middlewares: [BearerAuthMiddleware(provider: tokenProvider), ErrorMappingMiddleware()]
        )
    }

    public func ping() async throws {
        _ = try await tags()
    }

    public func documents(matching query: String) async throws -> [Document] {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return try await call {
            if trimmed.isEmpty {
                return try await client.listDocuments().ok.body.json.documents.map { try Document($0) }
            }
            let results = try await client.searchDocuments(query: .init(q: trimmed, limit: 50))
            return try results.ok.body.json.results.map { try Document($0.document) }
        }
    }

    public func deleteDocument(id: Document.ID) async throws {
        try await call {
            _ = try await client.deleteDocument(path: .init(id: id.uuidString.lowercased())).ok
        }
    }

    public func tags() async throws -> [Tag] {
        try await call {
            try await client.listTags().ok.body.json.map {
                try Tag(id: Self.uuid($0.id), name: $0.name, color: $0.color)
            }
        }
    }

    public func correspondents() async throws -> [Correspondent] {
        try await call {
            try await client.listCorrespondents().ok.body.json.map {
                try Correspondent(id: Self.uuid($0.id), name: $0.name)
            }
        }
    }

    public func documentTypes() async throws -> [DocumentType] {
        try await call {
            try await client.listDocumentTypes().ok.body.json.map {
                try DocumentType(id: Self.uuid($0.id), name: $0.name)
            }
        }
    }

    public func downloadFile(for document: Document) async throws -> URL {
        let request = await authorized(url: endpoint("api/documents/\(document.id.uuidString.lowercased())/download"))
        let (temporary, response) = try await transfer { try await session.download(for: request) }
        try check(response, body: try? Data(contentsOf: temporary))

        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-\(document.id.uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let destination = directory.appending(
            path: Self.safeFilename(document.originalFilename, mimeType: document.mimeType))
        try? FileManager.default.removeItem(at: destination)
        try FileManager.default.moveItem(at: temporary, to: destination)
        return destination
    }

    public func upload(_ file: UploadFile) async throws -> Document {
        var request = await authorized(url: endpoint("api/documents/upload"))
        request.httpMethod = "POST"
        let form = MultipartForm(file: file)
        request.setValue(form.contentType, forHTTPHeaderField: "Content-Type")

        let (data, response) = try await transfer { try await session.upload(for: request, from: form.body) }
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
        if !["", ".", "..", "/"].contains(last) { return last }
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

    private func authorized(url: URL) async -> URLRequest {
        var request = URLRequest(url: url)
        if let token = await tokenProvider.token() {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
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
        guard (200..<300).contains(http.statusCode) else {
            throw FolioError.server(status: http.statusCode, message: body.flatMap(ErrorMappingMiddleware.message))
        }
    }

    private func call<T>(_ work: () async throws -> T) async throws -> T {
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
            pageCount: raw.pageCount.map(Int.init), createdAt: raw.createdAt, updatedAt: raw.updatedAt
        )
    }
}
