import AppKit
import FileProvider
import FolioKit
import XCTest

private let thumbnailVault = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
private let thumbnailDocument = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!

private struct ThumbnailToken: BearerTokenProvider {
    func token() async throws -> String? { "thumbnail-test-token" }
}

private final class ThumbnailProtocol: URLProtocol {
    static let lock = NSLock()
    static var handlers: [String: (ThumbnailProtocol) -> Void] = [:]
    static var stopped: [String: () -> Void] = [:]

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {
        Self.lock.lock()
        let stopped = Self.stopped.removeValue(forKey: request.url!.host!)
        Self.lock.unlock()
        stopped?()
    }
    override func startLoading() {
        Self.lock.lock()
        let handler = Self.handlers[request.url!.host!]
        Self.lock.unlock()
        handler?(self)
    }

    func respond(_ status: Int, data: Data = Data()) {
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": "image/jpeg"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
}

final class ThumbnailTests: XCTestCase {
    private var session: URLSession!
    private var host: String!
    private var provider: FolioFileProvider!

    override func setUp() {
        host = UUID().uuidString.lowercased() + ".invalid"
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ThumbnailProtocol.self]
        session = URLSession(configuration: configuration)
        provider = FolioFileProvider(
            domain: NSFileProviderDomain(identifier: .init("thumbnail-test"), displayName: "Test"),
            service: LiveFolioService(
                baseURL: URL(string: "https://\(host!)/")!, tokenProvider: ThumbnailToken(), session: session)
        )
    }

    override func tearDown() {
        session.invalidateAndCancel()
        ThumbnailProtocol.lock.lock()
        ThumbnailProtocol.handlers.removeValue(forKey: host)
        ThumbnailProtocol.stopped.removeValue(forKey: host)
        ThumbnailProtocol.lock.unlock()
    }

    private func handle(_ body: @escaping (ThumbnailProtocol) -> Void) {
        ThumbnailProtocol.lock.lock()
        ThumbnailProtocol.handlers[host] = body
        ThumbnailProtocol.lock.unlock()
    }

    private func identifier(_ id: UUID = thumbnailDocument) -> NSFileProviderItemIdentifier {
        FolioItemID.document(id, in: .scoped(thumbnailVault, .all)).providerIdentifier
    }

    func testBatchReturnsScopedThumbnailMissingFolderAndFailureIndependently() async {
        let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: 1, pixelsHigh: 1, bitsPerSample: 8,
            samplesPerPixel: 3, hasAlpha: false, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 3, bitsPerPixel: 24
        )!
        bitmap.setColor(.white, atX: 0, y: 0)
        let jpeg = bitmap.representation(using: .jpeg, properties: [:])!
        let missingID = UUID()
        let missing = identifier(missingID)
        let denied = identifier(UUID())
        let folder = NSFileProviderItemIdentifier.rootContainer
        let success = identifier()
        handle { transfer in
            XCTAssertEqual(
                transfer.request.value(forHTTPHeaderField: "X-Folio-Vault"), thumbnailVault.uuidString.lowercased())
            XCTAssertEqual(transfer.request.value(forHTTPHeaderField: "Authorization"), "Bearer thumbnail-test-token")
            if transfer.request.url!.path.contains(thumbnailDocument.uuidString.lowercased()) {
                transfer.respond(200, data: jpeg)
            } else if transfer.request.url!.path.contains(missingID.uuidString.lowercased()) {
                transfer.respond(404)
            } else {
                transfer.respond(403)
            }
        }
        let completed = expectation(description: "Batch completed")
        var received: [NSFileProviderItemIdentifier] = []
        let progress = provider.fetchThumbnails(
            for: [success, missing, folder, denied], requestedSize: CGSize(width: 128, height: 128)
        ) { item, data, error in
            received.append(item)
            if item == success {
                XCTAssertEqual(data, jpeg)
                XCTAssertNil(error)
            } else if item == denied {
                XCTAssertNil(data)
                XCTAssertEqual((error as NSError?)?.code, NSFileReadNoPermissionError)
            } else {
                XCTAssertNil(data)
                XCTAssertNil(error)
            }
        } completionHandler: { error in
            XCTAssertNil(error)
            XCTAssertEqual(received, [success, missing, folder, denied])
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
        XCTAssertEqual(progress.completedUnitCount, 4)
    }

    func testUnauthorizedThumbnailReportsAuthenticationFailureForTheItem() async {
        handle { $0.respond(401) }
        let itemCompleted = expectation(description: "Authentication failure delivered")
        let completed = expectation(description: "Batch completed")
        _ = provider.fetchThumbnails(for: [identifier()], requestedSize: .zero) { _, data, error in
            XCTAssertNil(data)
            XCTAssertEqual((error as? NSFileProviderError)?.code, .notAuthenticated)
            itemCompleted.fulfill()
        } completionHandler: { error in
            XCTAssertNil(error)
            completed.fulfill()
        }
        await fulfillment(of: [itemCompleted, completed], timeout: 5, enforceOrder: true)
    }

    func testInvalidIdentifierReportsNoSuchItemWithoutNetworkRequest() async {
        handle { _ in XCTFail("Invalid identifiers must not reach the server") }
        let invalid = NSFileProviderItemIdentifier("not-a-document")
        let itemCompleted = expectation(description: "Invalid item rejected")
        let completed = expectation(description: "Batch completed")
        _ = provider.fetchThumbnails(for: [invalid], requestedSize: .zero) { item, data, error in
            XCTAssertEqual(item, invalid)
            XCTAssertNil(data)
            XCTAssertEqual((error as? NSFileProviderError)?.code, .noSuchItem)
            itemCompleted.fulfill()
        } completionHandler: { error in
            XCTAssertNil(error)
            completed.fulfill()
        }
        await fulfillment(of: [itemCompleted, completed], timeout: 5, enforceOrder: true)
    }

    func testEmptyBatchCompletesWithoutRequestingCredentials() async {
        handle { _ in XCTFail("Empty batch should not request a thumbnail") }
        let completed = expectation(description: "Empty batch completed")
        _ = provider.fetchThumbnails(for: [], requestedSize: .zero) { _, _, _ in
            XCTFail("Empty batch has no item callback")
        } completionHandler: { error in
            XCTAssertNil(error)
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
    }

    func testCancellationCompletesOnceWithoutWaitingForTheServer() async {
        let started = expectation(description: "Request started")
        let stopped = expectation(description: "Network transfer cancelled")
        ThumbnailProtocol.lock.lock()
        ThumbnailProtocol.stopped[host] = { stopped.fulfill() }
        ThumbnailProtocol.lock.unlock()
        handle { _ in started.fulfill() }
        let completed = expectation(description: "Cancellation completed")
        completed.assertForOverFulfill = true
        let progress = provider.fetchThumbnails(for: [identifier(), identifier(UUID())], requestedSize: .zero) {
            _, _, _ in
            XCTFail("Cancelled batch should not deliver item callbacks")
        } completionHandler: { error in
            XCTAssertEqual((error as NSError?)?.domain, NSCocoaErrorDomain)
            XCTAssertEqual((error as NSError?)?.code, NSUserCancelledError)
            completed.fulfill()
        }
        await fulfillment(of: [started], timeout: 5)
        progress.cancel()
        progress.cancel()
        await fulfillment(of: [completed, stopped], timeout: 5)
    }
}
