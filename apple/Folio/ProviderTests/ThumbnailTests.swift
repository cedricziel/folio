import AppKit
import FileProvider
import FolioKit
import XCTest
import os

private let thumbnailVault = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
private let thumbnailDocument = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!

private struct ThumbnailToken: BearerTokenProvider {
    func token() async throws -> String? {
        "thumbnail-test-token"
    }
}

private final class ThumbnailProtocol: URLProtocol {
    static let handlers = OSAllocatedUnfairLock<[String: @Sendable (ThumbnailProtocol) -> Void]>(initialState: [:])
    static let stopped = OSAllocatedUnfairLock<[String: @Sendable () -> Void]>(initialState: [:])

    override class func canInit(with _: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func stopLoading() {
        let stopped = Self.stopped.withLock { [host = request.url!.host!] in $0.removeValue(forKey: host) }
        stopped?()
    }

    override func startLoading() {
        let handler = Self.handlers.withLock { [host = request.url!.host!] in $0[host] }
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
                baseURL: URL(string: "https://\(host!)/")!, tokenProvider: ThumbnailToken(), session: session
            )
        )
    }

    override func tearDown() {
        session.invalidateAndCancel()
        ThumbnailProtocol.handlers.withLock { [host = host!] in _ = $0.removeValue(forKey: host) }
        ThumbnailProtocol.stopped.withLock { [host = host!] in _ = $0.removeValue(forKey: host) }
    }

    private func handle(_ body: @escaping @Sendable (ThumbnailProtocol) -> Void) {
        ThumbnailProtocol.handlers.withLock { [host = host!] in $0[host] = body }
    }

    private func identifier(_ id: UUID = thumbnailDocument) -> NSFileProviderItemIdentifier {
        FolioItemID.document(id, in: .scoped(thumbnailVault, .all)).providerIdentifier
    }

    func testBatchReturnsScopedThumbnailMissingFolderAndFailureIndependently() async throws {
        let bitmap = try XCTUnwrap(
            NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: 1, pixelsHigh: 1, bitsPerSample: 8,
                samplesPerPixel: 3, hasAlpha: false, isPlanar: false, colorSpaceName: .deviceRGB,
                bytesPerRow: 3, bitsPerPixel: 24
            ))
        bitmap.setColor(.white, atX: 0, y: 0)
        let jpeg = try XCTUnwrap(bitmap.representation(using: .jpeg, properties: [:]))
        let missingID = UUID()
        let missing = identifier(missingID)
        let denied = identifier(UUID())
        let folder = NSFileProviderItemIdentifier.rootContainer
        let success = identifier()
        handle { transfer in
            XCTAssertEqual(
                transfer.request.value(forHTTPHeaderField: "X-Folio-Vault"), thumbnailVault.uuidString.lowercased()
            )
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
        let received = OSAllocatedUnfairLock<[NSFileProviderItemIdentifier]>(initialState: [])
        let progress = provider.fetchThumbnails(
            for: [success, missing, folder, denied], requestedSize: CGSize(width: 128, height: 128)
        ) { @Sendable item, data, error in
            received.withLock { $0.append(item) }
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
        } completionHandler: { @Sendable error in
            XCTAssertNil(error)
            XCTAssertEqual(received.withLock { $0 }, [success, missing, folder, denied])
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
        XCTAssertEqual(progress.completedUnitCount, 4)
    }

    func testUnauthorizedThumbnailReportsAuthenticationFailureForTheItem() async {
        handle { $0.respond(401) }
        let itemCompleted = expectation(description: "Authentication failure delivered")
        let completed = expectation(description: "Batch completed")
        _ = provider.fetchThumbnails(for: [identifier()], requestedSize: .zero) { @Sendable _, data, error in
            XCTAssertNil(data)
            XCTAssertEqual((error as? NSFileProviderError)?.code, .notAuthenticated)
            itemCompleted.fulfill()
        } completionHandler: { @Sendable error in
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
        _ = provider.fetchThumbnails(for: [invalid], requestedSize: .zero) { @Sendable item, data, error in
            XCTAssertEqual(item, invalid)
            XCTAssertNil(data)
            XCTAssertEqual((error as? NSFileProviderError)?.code, .noSuchItem)
            itemCompleted.fulfill()
        } completionHandler: { @Sendable error in
            XCTAssertNil(error)
            completed.fulfill()
        }
        await fulfillment(of: [itemCompleted, completed], timeout: 5, enforceOrder: true)
    }

    func testEmptyBatchCompletesWithoutRequestingCredentials() async {
        handle { _ in XCTFail("Empty batch should not request a thumbnail") }
        let completed = expectation(description: "Empty batch completed")
        _ = provider.fetchThumbnails(for: [], requestedSize: .zero) { @Sendable _, _, _ in
            XCTFail("Empty batch has no item callback")
        } completionHandler: { @Sendable error in
            XCTAssertNil(error)
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
    }

    func testCancellationCompletesOnceWithoutWaitingForTheServer() async {
        let started = expectation(description: "Request started")
        let stopped = expectation(description: "Network transfer cancelled")
        ThumbnailProtocol.stopped.withLock { [host = host!] in $0[host] = { stopped.fulfill() } }
        handle { _ in started.fulfill() }
        let completed = expectation(description: "Cancellation completed")
        completed.assertForOverFulfill = true
        let progress = provider.fetchThumbnails(for: [identifier(), identifier(UUID())], requestedSize: .zero) {
            @Sendable _, _, _ in
            XCTFail("Cancelled batch should not deliver item callbacks")
        } completionHandler: { @Sendable error in
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
