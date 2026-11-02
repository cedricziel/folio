import FileProvider
import FolioKit
import UniformTypeIdentifiers
import XCTest

private final class UploadTemplate: NSObject, NSFileProviderItem {
    var itemIdentifier = NSFileProviderItemIdentifier("new-file")
    var parentItemIdentifier = FolioItemID.folder(.scoped(MockFolioService.defaultVault.id, .inbox)).providerIdentifier
    var filename = "scan.pdf"
    var contentType = UTType.pdf
}

final class FileProviderTests: XCTestCase {
    func testUploadCapturesTheVaultInboxEvenIfTheTemplateAndDefaultChange() async throws {
        let personal = Vault(id: UUID(), name: "Personal", role: .owner)
        let shared = Vault(id: UUID(), name: "Shared", role: .contributor)
        let personalService = MockFolioService()
        let sharedService = MockFolioService()
        let source = MockFolioService(
            vaults: VaultCatalog(vaults: [personal, shared], defaultVaultID: personal.id),
            vaultServices: [personal.id: personalService, shared.id: sharedService]
        )
        let provider = FolioFileProvider(
            domain: NSFileProviderDomain(identifier: .init("scoped-upload-test"), displayName: "Test"), service: source
        )
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
        try Data("%PDF-1.4".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        let template = UploadTemplate()
        template.parentItemIdentifier = FolioItemID.folder(.scoped(shared.id, .inbox)).providerIdentifier
        await sharedService.suspendUploads()
        let completed = expectation(description: "Scoped upload completes")

        _ = provider.createItem(
            basedOn: template, fields: [.contents], contents: file, request: NSFileProviderRequest()
        ) {
            item, _, _, error in
            XCTAssertNil(error)
            XCTAssertEqual(
                item?.parentItemIdentifier, FolioItemID.folder(.scoped(shared.id, .inbox)).providerIdentifier)
            completed.fulfill()
        }
        await sharedService.waitForUploadRequest()
        template.parentItemIdentifier = FolioItemID.folder(.scoped(personal.id, .inbox)).providerIdentifier
        await source.setVaults(VaultCatalog(vaults: [personal, shared], defaultVaultID: shared.id))
        await sharedService.releaseUploads()
        await fulfillment(of: [completed], timeout: 5)

        let uploaded = await sharedService.uploads
        let unrelated = await personalService.uploads
        XCTAssertEqual(uploaded.map(\.filename), ["scan.pdf"])
        XCTAssertTrue(unrelated.isEmpty)
    }

    func testProviderRootDoesNotChooseAnArbitraryUploadVault() async {
        let source = MockFolioService()
        let provider = FolioFileProvider(
            domain: NSFileProviderDomain(identifier: .init("root-upload-test"), displayName: "Test"), service: source
        )
        let template = UploadTemplate()
        template.parentItemIdentifier = .rootContainer
        let completed = expectation(description: "Root upload rejected")

        _ = provider.createItem(
            basedOn: template, fields: [], contents: URL(filePath: "/unused"), request: NSFileProviderRequest()
        ) {
            item, _, _, error in
            XCTAssertNil(item)
            XCTAssertEqual((error as NSError?)?.code, NSFeatureUnsupportedError)
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
        let uploads = await source.uploads
        XCTAssertTrue(uploads.isEmpty)
    }

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
            XCTAssertEqual(
                item?.parentItemIdentifier,
                FolioItemID.folder(.scoped(MockFolioService.defaultVault.id, .inbox)).providerIdentifier)
            XCTAssertTrue(pending.isEmpty)
            XCTAssertFalse(fetch)
            completed.fulfill()
        }
        await fulfillment(of: [completed], timeout: 5)
        let uploads = await service.uploads
        XCTAssertEqual(uploads.count, 1)
    }
}
