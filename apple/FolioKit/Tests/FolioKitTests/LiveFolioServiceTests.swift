@testable import FolioKit
import Foundation
import Testing

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
 "content": "Rechnungsnummer 2025-00318", "original_filename": "bill.pdf",
 "file_path": "a/bill.pdf", "mime_type": "application/pdf", "correspondent_id": null,
 "document_type_id": "11111111-2222-4333-8444-555555555555", "page_count": 3,
 "created_at": "2025-03-14T10:15:30.123456789Z", "updated_at": "2025-03-14T10:15:30Z"}
"""

@Suite(.serialized) struct LiveFolioServiceTests {
    private let baseURL = URL(string: "https://folio.example.com")!

    private func makeService(token: (any BearerTokenProvider)? = nil) -> LiveFolioService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubURLProtocol.self]
        return LiveFolioService(
            baseURL: baseURL, tokenProvider: token ?? NoToken(),
            session: URLSession(configuration: configuration)
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
            #expect(request.value(forHTTPHeaderField: "Content-Type")?.hasPrefix("multipart/form-data; boundary=") == true)
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

    @Test func downloadKeepsTheFileExtensionForQuickLook() async throws {
        StubURLProtocol.handler = { request, _ in
            #expect(request.url?.path.hasSuffix("/download") == true)
            return .init(contentType: "application/pdf", body: Data("%PDF-1.4".utf8))
        }
        let document = Document(title: "Bill", originalFilename: "bill.pdf", mimeType: "application/pdf")
        let url = try await makeService().downloadFile(for: document)
        #expect(url.pathExtension == "pdf")
        #expect(try Data(contentsOf: url) == Data("%PDF-1.4".utf8))
    }
}
