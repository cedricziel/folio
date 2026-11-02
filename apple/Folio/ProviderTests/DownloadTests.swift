import FileProvider
import FolioKit
import PDFKit
import XCTest

private let downloadID = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!
private let downloadVaultID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!

private final class TransferProtocol: URLProtocol {
    override class func canInit(with _: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func stopLoading() {}

    override func startLoading() {
        let download = request.url!.path.hasSuffix("/download")
        let discovery = request.url!.path.hasSuffix("/api/vaults")
        let missing = download && request.url!.path.contains("/missing/")
        let wrongVault =
            !discovery && request.value(forHTTPHeaderField: "X-Folio-Vault") != downloadVaultID.uuidString.lowercased()
        let status = missing || wrongVault ? 404 : 200
        let body: Data
        if discovery {
            body = Data(
                "{\"vaults\":[{\"id\":\"\(downloadVaultID)\",\"name\":\"Household\",\"role\":\"viewer\"}],\"default_vault_id\":\"\(downloadVaultID)\"}"
                    .utf8)
        } else if download {
            body =
                missing
                ? Data()
                : try! Data(
                    contentsOf: Bundle(for: DownloadTests.self).url(forResource: "sample", withExtension: "pdf")!
                )
        } else {
            body = Data(
                """
                {"documents":[{"id":"\(downloadID.uuidString.lowercased())","title":"Sample","original_filename":"sample.pdf",
                "mime_type":"application/pdf","vault_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee","uploaded_by":"test",
                "ocr_status":"done","embed_status":"done","tags":[],
                "created_at":"2025-01-01T00:00:00Z","updated_at":"2025-01-01T00:00:00Z"}],"count":1}
                """.utf8
            )
        }
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": download ? "application/pdf" : "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }
}

final class DownloadTests: XCTestCase {
    private var directory: URL!
    private var session: URLSession!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TransferProtocol.self]
        session = URLSession(configuration: config)
    }

    override func tearDown() {
        session.invalidateAndCancel()
        try? FileManager.default.removeItem(at: directory)
    }

    private func provider(path: String = "") -> FolioFileProvider {
        let service = LiveFolioService(
            baseURL: URL(string: "https://provider.invalid/" + path)!,
            session: session, downloadDirectory: directory
        )
        let staging = directory!
        return FolioFileProvider(
            domain: NSFileProviderDomain(identifier: .init("download-test"), displayName: "Test"),
            service: service, temporaryDirectory: { staging }
        )
    }

    private func fetch(_ provider: FolioFileProvider) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            _ = provider.fetchContents(
                for: FolioItemID.document(downloadID, in: .scoped(downloadVaultID, .all)).providerIdentifier,
                version: nil, request: NSFileProviderRequest()
            ) { url, item, error in
                if let error {
                    continuation.resume(throwing: error); return
                }
                XCTAssertEqual(
                    item?.itemIdentifier,
                    FolioItemID.document(downloadID, in: .scoped(downloadVaultID, .all)).providerIdentifier)
                XCTAssertEqual(item?.capabilities, [.allowsReading])
                guard let url else { continuation.resume(throwing: NSFileProviderError(.noSuchItem)); return }
                continuation.resume(returning: url)
            }
        }
    }

    func testReadOnlyDocumentDownloadsAsReadablePDFAndCanReopen() async throws {
        let provider = provider()
        let first = try await fetch(provider)
        XCTAssertEqual(PDFDocument(url: first)?.pageCount, 1)
        XCTAssertEqual(first.deletingLastPathComponent().standardizedFileURL, directory.standardizedFileURL)
        let original = try Data(contentsOf: first)
        let second = try await fetch(provider)
        XCTAssertNotEqual(first, second)
        XCTAssertEqual(try Data(contentsOf: first), original)
        XCTAssertEqual(try Data(contentsOf: second), original)
        XCTAssertEqual(PDFDocument(url: second)?.pageCount, 1)
    }

    func testMissingServerOriginalReturnsNoSuchItem() async {
        do {
            _ = try await fetch(provider(path: "missing"))
            XCTFail("Expected missing original to fail")
        } catch {
            XCTAssertEqual((error as? NSFileProviderError)?.code, .noSuchItem)
        }
    }
}
