import FileProvider
import FolioKit
import UniformTypeIdentifiers
import XCTest

private final class UploadTemplate: NSObject, NSFileProviderItem {
    var itemIdentifier = NSFileProviderItemIdentifier("new-file")
    var parentItemIdentifier = FolioItemID.folder(.inbox).providerIdentifier
    var filename = "scan.pdf"
    var contentType = UTType.pdf
}

final class FileProviderTests: XCTestCase {
    func testUploadReturnsAnItemWhileProcessingHasNotStarted() async throws {
        let service = MockFolioService()
        let domain = NSFileProviderDomain(identifier: .init("upload-test"), displayName: "Test")
        let provider = FolioFileProvider(domain: domain, service: service)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
        try Data("%PDF-1.4".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        let completed = expectation(description: "Upload completes")

        _ = provider.createItem(
            basedOn: UploadTemplate(), fields: [.contents, .filename], contents: file,
            request: NSFileProviderRequest()
        ) { item, pending, fetch, error in
            XCTAssertNil(error)
            XCTAssertNotNil(item)
            XCTAssertEqual(item?.filename, "scan.pdf")
            XCTAssertEqual(item?.parentItemIdentifier, FolioItemID.folder(.inbox).providerIdentifier)
            XCTAssertTrue(pending.isEmpty)
            XCTAssertFalse(fetch)
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
        let uploads = await service.uploads
        XCTAssertEqual(uploads.count, 1)
    }
}
