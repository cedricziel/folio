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

private final class EditedItem: NSObject, NSFileProviderItem {
    let itemIdentifier: NSFileProviderItemIdentifier
    var parentItemIdentifier: NSFileProviderItemIdentifier
    var filename = "bill.pdf"
    var lastUsedDate: Date?
    var tagData: Data?
    var favoriteRank: NSNumber?

    init(_ id: FolioItemID, parent: FolioItemID) {
        itemIdentifier = id.providerIdentifier
        parentItemIdentifier = parent.providerIdentifier
    }
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

    private static let anyVersion = NSFileProviderItemVersion(contentVersion: Data(), metadataVersion: Data())

    func testLocalMetadataIsKeptAndReportedWithTheItem() async throws {
        let document = Document(title: "Bill", originalFilename: "bill.pdf")
        let service = MockFolioService(documents: [document])
        let folder = FolioItemID.Folder.scoped(MockFolioService.defaultVault.id, .all)
        let id = FolioItemID.document(document.id, in: folder)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".json")
        defer { try? FileManager.default.removeItem(at: file) }
        let domain = NSFileProviderDomain(identifier: .init("local-metadata-test"), displayName: "Test")
        let provider = FolioFileProvider(
            domain: domain, service: service, localMetadata: LocalItemMetadataStore(file: file))
        let edited = EditedItem(id, parent: .folder(folder))
        edited.lastUsedDate = Date(timeIntervalSince1970: 1_750_000_000)
        edited.tagData = Data("red".utf8)
        edited.favoriteRank = NSNumber(value: UInt64(7))
        let modified = expectation(description: "Local metadata saved")

        _ = provider.modifyItem(
            edited, baseVersion: Self.anyVersion, changedFields: LocalItemMetadata.fields, contents: nil,
            request: NSFileProviderRequest()
        ) { item, pending, fetch, error in
            XCTAssertNil(error)
            XCTAssertEqual(item?.lastUsedDate ?? nil, edited.lastUsedDate)
            XCTAssertEqual(item?.tagData ?? nil, edited.tagData)
            XCTAssertEqual(item?.favoriteRank ?? nil, edited.favoriteRank)
            XCTAssertTrue(pending.isEmpty)
            XCTAssertFalse(fetch)
            modified.fulfill()
        }
        await fulfillment(of: [modified], timeout: 5)

        let restarted = FolioFileProvider(
            domain: domain, service: service, localMetadata: LocalItemMetadataStore(file: file))
        let looked = expectation(description: "Item looked up")
        _ = restarted.item(for: id.providerIdentifier, request: NSFileProviderRequest()) { item, error in
            XCTAssertNil(error)
            XCTAssertEqual(item?.tagData ?? nil, Data("red".utf8))
            XCTAssertEqual(item?.lastUsedDate ?? nil, edited.lastUsedDate)
            looked.fulfill()
        }
        await fulfillment(of: [looked], timeout: 5)
    }

    func testRenamingIsStillRefused() async {
        let document = Document(title: "Bill", originalFilename: "bill.pdf")
        let folder = FolioItemID.Folder.scoped(MockFolioService.defaultVault.id, .all)
        let provider = FolioFileProvider(
            domain: NSFileProviderDomain(identifier: .init("rename-test"), displayName: "Test"),
            service: MockFolioService(documents: [document]))
        let edited = EditedItem(.document(document.id, in: folder), parent: .folder(folder))
        edited.filename = "renamed.pdf"
        let refused = expectation(description: "Rename refused")

        _ = provider.modifyItem(
            edited, baseVersion: Self.anyVersion, changedFields: [.filename, .lastUsedDate], contents: nil,
            request: NSFileProviderRequest()
        ) { item, _, _, error in
            XCTAssertNil(item)
            XCTAssertEqual((error as NSError?)?.code, NSFeatureUnsupportedError)
            refused.fulfill()
        }
        await fulfillment(of: [refused], timeout: 5)
    }

    #if os(macOS)
        func testKeepDownloadedIsStoredAndReportedAsContentPolicy() async throws {
            let document = Document(title: "Bill", originalFilename: "bill.pdf")
            let folder = FolioItemID.Folder.scoped(MockFolioService.defaultVault.id, .all)
            let id = FolioItemID.document(document.id, in: folder).providerIdentifier
            let provider = FolioFileProvider(
                domain: NSFileProviderDomain(identifier: .init("keep-downloaded-test"), displayName: "Test"),
                service: MockFolioService(documents: [document]))

            let before = try await lookUp(id, in: provider)
            XCTAssertEqual(before.contentPolicy, .inherited)
            XCTAssertEqual(before.userInfo??["keptDownloaded"] as? Bool, false)

            try await perform(FolioProviderItem.keepDownloadedAction, on: [id], in: provider)
            let kept = try await lookUp(id, in: provider)
            XCTAssertEqual(kept.contentPolicy, .downloadEagerlyAndKeepDownloaded)
            XCTAssertEqual(kept.userInfo??["keptDownloaded"] as? Bool, true)
            XCTAssertNotEqual(kept.itemVersion?.metadataVersion, before.itemVersion?.metadataVersion)

            try await perform(FolioProviderItem.allowEvictionAction, on: [id], in: provider)
            let released = try await lookUp(id, in: provider)
            XCTAssertEqual(released.contentPolicy, .inherited)
            XCTAssertEqual(released.itemVersion?.metadataVersion, before.itemVersion?.metadataVersion)
        }

        func testUnknownActionsAreRefused() async {
            let provider = FolioFileProvider(
                domain: NSFileProviderDomain(identifier: .init("unknown-action-test"), displayName: "Test"),
                service: MockFolioService())
            do {
                try await perform(.init("com.example.unknown"), on: [.rootContainer], in: provider)
                XCTFail("Unknown action was performed")
            } catch {
                XCTAssertEqual((error as NSError).code, NSFeatureUnsupportedError)
            }
        }

        private func lookUp(
            _ id: NSFileProviderItemIdentifier, in provider: FolioFileProvider
        ) async throws -> NSFileProviderItem {
            try await withCheckedThrowingContinuation { continuation in
                _ = provider.item(for: id, request: NSFileProviderRequest()) { item, error in
                    if let item {
                        continuation.resume(returning: item)
                    } else {
                        continuation.resume(throwing: error ?? NSFileProviderError(.noSuchItem))
                    }
                }
            }
        }

        private func perform(
            _ action: NSFileProviderExtensionActionIdentifier, on ids: [NSFileProviderItemIdentifier],
            in provider: FolioFileProvider
        ) async throws {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                _ = provider.performAction(identifier: action, onItemsWithIdentifiers: ids) { error in
                    if let error {
                        continuation.resume(throwing: error)
                    } else {
                        continuation.resume()
                    }
                }
            }
        }
    #endif
}
