import Foundation
import Testing

@testable import FolioKit

final class StubURLProtocol: URLProtocol, @unchecked Sendable {
    struct Reply {
        var status = 200
        var contentType = "application/json"
        var body = Data()
    }

    nonisolated(unsafe) static var handler: ((URLRequest, Data?) -> Reply)?

    override class func canInit(with _: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        let body = request.httpBody ?? request.httpBodyStream.map(Self.drain)
        let reply = Self.handler?(request, body) ?? Reply(status: 500)
        let response = HTTPURLResponse(
            url: request.url!, statusCode: reply.status, httpVersion: nil,
            headerFields: ["Content-Type": reply.contentType]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: reply.body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func drain(_ stream: InputStream) -> Data {
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 {
                break
            }
            data.append(buffer, count: count)
        }
        return data
    }
}

private struct FixedToken: BearerTokenProvider {
    let value: String
    func token() async -> String? {
        value
    }
}

private let documentJSON = """
    {"id": "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f", "title": "Jahresabrechnung 2025",
     "vault_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "uploaded_by": "fixture-user",
     "content": "Rechnungsnummer 2025-00318", "original_filename": "bill.pdf",
     "file_path": "a/bill.pdf", "mime_type": "application/pdf", "correspondent_id": null,
     "document_type_id": "11111111-2222-4333-8444-555555555555", "page_count": 3,
     "ocr_status": "done", "embed_status": "skipped",
     "correspondent": null, "document_type": "Invoice",
     "tags": [{"id": "99999999-8888-4777-8666-555555555555", "name": "Taxes", "color": "#ff5733"}],
     "created_at": "2025-03-14T10:15:30.123456789Z", "updated_at": "2025-03-14T10:15:30Z"}
    """

@Suite(.serialized) struct LiveFolioServiceTests {
    private let baseURL = URL(string: "https://folio.example.com")!

    @Test func browseSendsTextFiltersCursorAndImmutableVault() async throws {
        let vault = UUID(), tag = UUID(), correspondent = UUID()
        let query = DocumentBrowseQuery(
            text: "insurance policy", filter: .init(tagIDs: [tag], correspondentID: correspondent), sort: .title
        )
        StubURLProtocol.handler = { request, _ in
            #expect(request.url?.path == "/api/documents/browse")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)!.queryItems ?? []
            let value = { (name: String) in items.first { $0.name == name }?.value }
            #expect(value("q") == "insurance policy")
            #expect(value("limit") == "50")
            #expect(value("sort") == "title")
            #expect(value("cursor") == "next-token")
            #expect(value("tag_id") == tag.uuidString.lowercased())
            #expect(value("correspondent_id") == correspondent.uuidString.lowercased())
            return .init(
                body: Data("{\"vault_id\":\"\(vault)\",\"documents\":[\(documentJSON)],\"next_cursor\":\"more\"}".utf8))
        }
        let page = try await makeService().scoped(to: vault).documentPage(query: query, cursor: "next-token")
        #expect(page.vaultID == vault)
        #expect(page.nextCursor == "more")
        #expect(page.documents.first?.tags.first?.name == "Taxes")
    }

    @Test func directDocumentLookupKeepsVaultScopeAndTags() async throws {
        let vault = UUID()
        let id = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!
        StubURLProtocol.handler = { request, _ in
            #expect(request.url?.path == "/api/documents/\(id.uuidString.lowercased())")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            return .init(body: Data(documentJSON.utf8))
        }
        let document = try await makeService().scoped(to: vault).document(id: id)
        #expect(document.tags.first?.name == "Taxes")
    }

    @Test func discoversVaultNamesRolesAndServerDefault() async throws {
        let id = UUID()
        StubURLProtocol.handler = { request, _ in
            #expect(request.url?.path == "/api/vaults")
            return .init(
                body: Data(
                    """
                    {"vaults":[{"id":"\(id)","name":"Household","role":"contributor"}],"default_vault_id":"\(id)"}
                    """.utf8
                )
            )
        }
        let catalog = try await makeService().vaults()
        #expect(catalog.defaultVaultID == id)
        #expect(catalog.vaults == [Vault(id: id, name: "Household", role: .contributor)])
    }

    @Test func missingVaultDiscoveryIsAnError() async throws {
        StubURLProtocol.handler = { _, _ in .init(status: 404) }
        await #expect(throws: FolioError.server(status: 404, message: nil)) {
            _ = try await makeService().vaults()
        }
    }

    @Test func scopedCopiesKeepTheirOwnVaultOnConcurrentGeneratedRequests() async throws {
        let personal = UUID()
        let shared = UUID()
        let service = makeService()
        let personalService = service.scoped(to: personal)
        let sharedService = service.scoped(to: shared)
        StubURLProtocol.handler = { request, _ in
            let vault = request.value(forHTTPHeaderField: "X-Folio-Vault") ?? "default"
            return .init(
                body: Data(
                    """
                    [{"id":"\(UUID())","name":"\(vault)","color":null,"created_at":"2025-03-14T10:15:30Z"}]
                    """.utf8))
        }

        async let personalTags = personalService.tags()
        async let sharedTags = sharedService.tags()
        async let defaultTags = service.tags()
        let (personalResult, sharedResult, defaultResult) = try await (personalTags, sharedTags, defaultTags)

        #expect(personalResult.map(\.name) == [personal.uuidString.lowercased()])
        #expect(sharedResult.map(\.name) == [shared.uuidString.lowercased()])
        #expect(defaultResult.map(\.name) == ["default"])
    }

    @Test func scopedUploadRetainsItsDestinationHeader() async throws {
        let vault = UUID()
        StubURLProtocol.handler = { request, _ in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            return .init(body: Data(documentJSON.utf8))
        }
        _ = try await makeService().scoped(to: vault).upload(
            UploadFile(data: Data(), filename: "letter.pdf", mimeType: "application/pdf")
        )
    }

    @Test(arguments: [403, 404]) func scopedFailuresNeverFallBackToTheDefaultVault(status: Int) async {
        StubURLProtocol.handler = { _, _ in .init(status: status) }
        await #expect(throws: FolioError.server(status: status, message: nil)) {
            _ = try await makeService().scoped(to: UUID()).tags()
        }
    }

    private func makeService(token: (any BearerTokenProvider)? = nil, downloadDirectory: URL? = nil) -> LiveFolioService
    {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubURLProtocol.self]
        return LiveFolioService(
            baseURL: baseURL, tokenProvider: token ?? NoToken(),
            session: URLSession(configuration: configuration), downloadDirectory: downloadDirectory
        )
    }

    @Test func listsDocumentsAndMapsFields() async throws {
        StubURLProtocol.handler = { request, _ in
            #expect(request.url?.path == "/api/documents")
            return .init(body: Data("{\"documents\": [\(documentJSON)], \"count\": 1}".utf8))
        }
        let documents = try await makeService().documents(matching: "")
        let document = try #require(documents.first)
        #expect(document.title == "Jahresabrechnung 2025")
        #expect(document.pageCount == 3)
        #expect(document.correspondentID == nil)
        #expect(document.documentTypeID == UUID(uuidString: "11111111-2222-4333-8444-555555555555"))
        #expect(document.createdAt.timeIntervalSince1970 > 1_741_000_000)
        #expect(document.tags.map(\.name) == ["Taxes"])
        #expect(document.tags.first?.color == "#ff5733")
    }

    @Test func completedOcrWithoutTextDoesNotKeepReading() async throws {
        let json = documentJSON.replacingOccurrences(
            of: "\"content\": \"Rechnungsnummer 2025-00318\"", with: "\"content\": null"
        )
        StubURLProtocol.handler = { _, _ in
            .init(body: Data("{\"documents\": [\(json)], \"count\": 1}".utf8))
        }
        let document = try #require(try await makeService().documents(matching: "").first)
        #expect(document.status == .filed)
    }

    @Test(arguments: ["pending", "running"])
    func activeOcrWithExistingTextKeepsReading(stage: String) async throws {
        let json = documentJSON.replacingOccurrences(
            of: "\"ocr_status\": \"done\"", with: "\"ocr_status\": \"\(stage)\""
        )
        StubURLProtocol.handler = { _, _ in
            .init(body: Data("{\"documents\": [\(json)], \"count\": 1}".utf8))
        }
        let document = try #require(try await makeService().documents(matching: "").first)
        #expect(document.status == .reading(pages: 3))
    }

    @Test(arguments: ["pending", "running"])
    func activeEmbeddingIsIndexing(stage: String) async throws {
        let json = documentJSON.replacingOccurrences(
            of: "\"embed_status\": \"skipped\"", with: "\"embed_status\": \"\(stage)\""
        )
        StubURLProtocol.handler = { _, _ in
            .init(body: Data("{\"results\": [{\"document\": \(json), \"score\": 0.9}]}".utf8))
        }
        let document = try #require(try await makeService().documents(matching: "invoice").first)
        #expect(document.status == .indexing)
        #expect(FolioFormat.statusText(document.status) == "Indexing…")
    }

    @Test func failedOcrIsNotStillReading() async throws {
        let json = documentJSON.replacingOccurrences(
            of: "\"ocr_status\": \"done\"", with: "\"ocr_status\": \"failed\""
        )
        .replacingOccurrences(of: "\"embed_status\": \"skipped\"", with: "\"embed_status\": \"pending\"")
        StubURLProtocol.handler = { _, _ in
            .init(body: Data("{\"documents\": [\(json)], \"count\": 1}".utf8))
        }
        let document = try #require(try await makeService().documents(matching: "").first)
        #expect(document.status == .failed)
        #expect(FolioFormat.statusText(document.status) == "Couldn't process")
    }

    @Test func failedEmbeddingIsNotFiled() async throws {
        let json = documentJSON.replacingOccurrences(
            of: "\"embed_status\": \"skipped\"", with: "\"embed_status\": \"failed\""
        )
        StubURLProtocol.handler = { _, _ in
            .init(body: Data("{\"documents\": [\(json)], \"count\": 1}".utf8))
        }
        let document = try #require(try await makeService().documents(matching: "").first)
        #expect(document.status == .failed)
    }

    @Test func sendsEveryFilterAsAQueryParameter() async throws {
        let tags = [UUID(), UUID()]
        let after = Date(timeIntervalSince1970: 1_741_910_400)
        StubURLProtocol.handler = { request, _ in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            func values(_ name: String) -> [String] {
                items.filter { $0.name == name }.compactMap(\.value)
            }
            #expect(request.url?.path == "/api/documents")
            #expect(values("tag_id") == tags.map { $0.uuidString.lowercased() })
            #expect(values("correspondent_id").count == 1)
            #expect(values("created_after").first?.hasPrefix("2025-03-14") == true)
            #expect(values("created_before").count == 1)
            return .init(body: Data("{\"documents\": [], \"count\": 0}".utf8))
        }
        let filter = DocumentFilter(
            tagIDs: tags, correspondentID: UUID(), createdAfter: after, createdBefore: after.addingTimeInterval(86400)
        )
        #expect(try await makeService().documents(filter: filter).isEmpty)
    }

    @Test func searchUsesTheSearchEndpointAndKeepsServerOrder() async throws {
        StubURLProtocol.handler = { request, _ in
            #expect(request.url?.path == "/api/documents/search")
            #expect(request.url?.query?.contains("q=strom") == true)
            return .init(
                body: Data("{\"results\": [{\"document\": \(documentJSON), \"score\": 0.9}]}".utf8)
            )
        }
        let documents = try await makeService().documents(matching: "strom")
        #expect(documents.map(\.title) == ["Jahresabrechnung 2025"])
    }

    @Test func sendsBearerTokenWhenConfigured() async throws {
        StubURLProtocol.handler = { request, _ in
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer secret")
            return .init(body: Data("[]".utf8))
        }
        _ = try await makeService(token: FixedToken(value: "secret")).tags()
    }

    @Test func removesATagWithAPostToTheTagPath() async throws {
        let tag = UUID()
        let document = UUID()
        let vault = UUID()
        StubURLProtocol.handler = { request, _ in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            #expect(request.httpMethod == "POST")
            #expect(
                request.url?.path
                    == "/api/documents/\(document.uuidString.lowercased())/tags/\(tag.uuidString.lowercased())"
            )
            return .init(contentType: "text/html")
        }
        try await makeService().scoped(to: vault).removeTag(tag, from: document)
    }

    @Test func addsATagWithinTheSelectedVault() async throws {
        let vault = UUID()
        let tag = UUID()
        let document = UUID()
        StubURLProtocol.handler = { request, body in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/documents/\(document.uuidString.lowercased())/tags")
            #expect(String(decoding: body ?? Data(), as: UTF8.self) == "tag_id=\(tag.uuidString.lowercased())")
            return .init(contentType: "text/html")
        }

        try await makeService().scoped(to: vault).addTag(tag, to: document)
    }

    @Test func surfacesServerErrorMessage() async throws {
        StubURLProtocol.handler = { _, _ in
            .init(status: 404, body: Data("{\"error\": \"Document not found\"}".utf8))
        }
        await #expect(throws: FolioError.server(status: 404, message: "Document not found")) {
            _ = try await makeService().documents(matching: "")
        }
    }

    @Test func uploadsMultipartWithFileMimeType() async throws {
        StubURLProtocol.handler = { request, body in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/documents/upload")
            #expect(
                request.value(forHTTPHeaderField: "Content-Type")?.hasPrefix("multipart/form-data; boundary=") == true
            )
            let text = String(decoding: body ?? Data(), as: UTF8.self)
            #expect(text.contains("name=\"file\"; filename=\"scan.pdf\""))
            #expect(text.contains("Content-Type: application/pdf"))
            #expect(text.contains("name=\"title\""))
            return .init(body: Data(documentJSON.utf8))
        }
        let file = UploadFile(
            data: Data("%PDF-1.4".utf8), filename: "scan.pdf", mimeType: "application/pdf",
            title: "Scan"
        )
        let document = try await makeService().upload(file)
        #expect(document.originalFilename == "bill.pdf")
    }

    @Test func repeatedDownloadsKeepEarlierContentsReadable() async throws {
        let document = Document(title: "Bill", originalFilename: "bill.pdf", mimeType: "application/pdf")
        let service = makeService()
        StubURLProtocol.handler = { _, _ in
            .init(contentType: "application/pdf", body: Data("first version".utf8))
        }
        let first = try await service.downloadFile(for: document)
        defer { try? FileManager.default.removeItem(at: first.deletingLastPathComponent()) }
        StubURLProtocol.handler = { _, _ in
            .init(contentType: "application/pdf", body: Data("second version".utf8))
        }
        let second = try await service.downloadFile(for: document)
        defer { try? FileManager.default.removeItem(at: second.deletingLastPathComponent()) }

        #expect(first != second)
        #expect(try Data(contentsOf: first) == Data("first version".utf8))
        #expect(try Data(contentsOf: second) == Data("second version".utf8))
    }

    @Test func providerDownloadsUseDistinctShortPathsInTheSpecifiedDirectory() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        StubURLProtocol.handler = { _, _ in
            .init(contentType: "application/pdf", body: Data("%PDF-1.4".utf8))
        }
        let document = Document(
            title: "Bill", originalFilename: String(repeating: "a", count: 240) + ".pdf", mimeType: "application/pdf"
        )
        let service = makeService(downloadDirectory: directory)
        async let firstDownload = service.downloadFile(for: document)
        async let secondDownload = service.downloadFile(for: document)
        let (first, second) = try await (firstDownload, secondDownload)

        #expect(first != second)
        #expect(first.deletingLastPathComponent().standardizedFileURL == directory.standardizedFileURL)
        #expect(second.deletingLastPathComponent().standardizedFileURL == directory.standardizedFileURL)
        #expect(first.pathExtension == "pdf")
        #expect(try Data(contentsOf: first) == Data("%PDF-1.4".utf8))
        #expect(try Data(contentsOf: second) == Data("%PDF-1.4".utf8))
        try FileManager.default.removeItem(at: first)
        #expect(try Data(contentsOf: second) == Data("%PDF-1.4".utf8))
    }

    @Test func downloadKeepsTheFileExtensionForQuickLook() async throws {
        let vault = UUID()
        StubURLProtocol.handler = { request, _ in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            #expect(request.url?.path.hasSuffix("/download") == true)
            return .init(contentType: "application/pdf", body: Data("%PDF-1.4".utf8))
        }
        let document = Document(title: "Bill", originalFilename: "bill.pdf", mimeType: "application/pdf")
        let url = try await makeService().scoped(to: vault).downloadFile(for: document)
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }
        #expect(url.pathExtension == "pdf")
        #expect(try Data(contentsOf: url) == Data("%PDF-1.4".utf8))
    }
}

struct SafeFilenameTests {
    @Test(arguments: [
        ("bill.pdf", "bill.pdf"),
        ("../../etc/passwd", "passwd"),
        ("/tmp/evil.pdf", "evil.pdf"),
        ("..", "document.pdf"),
        (".", "document.pdf"),
        ("", "document.pdf"),
    ])
    func keepsOnlyTheLastComponent(input: String, expected: String) {
        #expect(LiveFolioService.safeFilename(input, mimeType: "application/pdf") == expected)
    }

    @Test func fallsBackWithoutNameOrType() {
        #expect(LiveFolioService.safeFilename(nil, mimeType: nil) == "document")
    }
}
