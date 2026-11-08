import AppIntents
import Foundation
import Testing
import UniformTypeIdentifiers

@testable import FolioKit

private let server = URL(string: "https://folio.example")!
private let otherVault = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!

private func pdf(_ title: String, content: String? = nil, ocr: Document.ProcessingStatus = .pending) -> Document {
    Document(
        title: title, content: content, originalFilename: "\(title).pdf", mimeType: "application/pdf",
        ocrStatus: ocr)
}

struct DocumentTransferTypeTests {
    @Test func contentTypeFollowsTheOriginalsMimeType() {
        #expect(DocumentTransfer.contentType(mimeType: "application/pdf", filename: nil) == .pdf)
        #expect(DocumentTransfer.contentType(mimeType: "image/png", filename: "scan.png") == .png)
        #expect(DocumentTransfer.contentType(mimeType: "image/jpeg", filename: nil) == .jpeg)
        #expect(DocumentTransfer.contentType(mimeType: "IMAGE/HEIC", filename: nil) == .heic)
    }

    @Test func contentTypeFallsBackToTheFilenameThenToData() {
        #expect(DocumentTransfer.contentType(mimeType: nil, filename: "letter.pdf") == .pdf)
        #expect(DocumentTransfer.contentType(mimeType: "application/octet-stream", filename: "a.tiff") == .tiff)
        #expect(DocumentTransfer.contentType(mimeType: nil, filename: nil) == .data)
    }

    @Test func unknownMimeTypesDoNotHideTheFilenamesType() {
        #expect(DocumentTransfer.contentType(mimeType: "application/x-folio-unknown", filename: "letter.pdf") == .pdf)
        #expect(DocumentTransfer.contentType(mimeType: "application/x-folio-unknown", filename: "a.zzfolio") == .data)
    }

    @Test func everyExportedFileTypeIsOfferedOnceAndDataIsTheLastResort() {
        #expect(Set(DocumentTransfer.fileTypes).count == DocumentTransfer.fileTypes.count)
        #expect(DocumentTransfer.fileTypes.contains(.pdf))
        #expect(DocumentTransfer.exportedFileType(for: .pdf) == .pdf)
        #expect(DocumentTransfer.exportedFileType(for: .png) == .png)
        #expect(DocumentTransfer.exportedFileType(for: UTType("com.microsoft.word.doc")!) == .data)
    }

    @Test func entityCarriesTheContentTypeAndWhetherTextIsAvailable() {
        let scanned = DocumentEntity(
            DocumentSummary(pdf("Gas bill", ocr: .done), correspondent: nil, documentType: nil))
        #expect(scanned.contentType == .pdf)
        #expect(scanned.hasRecognizedText)

        let pending = DocumentEntity(DocumentSummary(pdf("Fresh"), correspondent: nil, documentType: nil))
        #expect(!pending.hasRecognizedText)

        let image = Document(title: "Receipt", originalFilename: "receipt.jpg", mimeType: "image/jpeg")
        #expect(DocumentEntity(DocumentSummary(image, correspondent: nil, documentType: nil)).contentType == .jpeg)
    }

    @Test func entityWebLinkPointsAtTheDocumentInItsVault() throws {
        let locator = DocumentLocator()
        let id = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!
        let document = "documents/3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f"
        #expect(locator.webURL(for: id) == nil)

        locator.reset(server: server)
        #expect(locator.webURL(for: id)?.absoluteString == "https://folio.example/\(document)")

        locator.record(documentID: id, vaultID: otherVault)
        let url = try #require(locator.webURL(for: id))
        #expect(url.absoluteString == "https://folio.example/vaults/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee/\(document)")

        locator.reset(server: nil)
        #expect(locator.webURL(for: id) == nil)
        #expect(locator.vault(for: id) == nil)
    }
}

struct DocumentTransferCatalogTests {
    @Test func exportedFileIsTheOriginalWithItsFilenameAndType() async throws {
        let target = pdf("Gas bill")
        let service = MockFolioService(documents: [target])
        let catalog = DocumentCatalog(service: service, locator: DocumentLocator())

        let file = try await catalog.transferFile(for: target.id)

        #expect(file.url.lastPathComponent == "Gas bill.pdf")
        #expect(file.contentType == .pdf)
        #expect(await service.downloads == 1)
    }

    @Test func exportDownloadsFromTheVaultTheDocumentWasOpenedIn() async throws {
        let target = pdf("Lease")
        let vaultService = MockFolioService(documents: [target])
        let service = MockFolioService(documents: [], vaultServices: [otherVault: vaultService])
        let locator = DocumentLocator()
        locator.reset(server: server)
        locator.record(documentID: target.id, vaultID: otherVault)

        let file = try await DocumentCatalog(service: service, locator: locator).transferFile(for: target.id)

        #expect(file.url.lastPathComponent == "Lease.pdf")
        #expect(await vaultService.downloads == 1)
        #expect(await service.downloads == 0)
    }

    @Test func exportOfAnInaccessibleOrTrashedDocumentFails() async {
        let trashed = Document(title: "Old", originalFilename: "old.pdf", deletedAt: Date(timeIntervalSince1970: 0))
        let catalog = DocumentCatalog(service: MockFolioService(documents: [trashed]), locator: DocumentLocator())

        await #expect(throws: ShortcutFailure.documentNotFound) { try await catalog.transferFile(for: UUID()) }
        await #expect(throws: ShortcutFailure.documentNotFound) { try await catalog.transferFile(for: trashed.id) }
    }

    @Test func serverFailuresAreNotReportedAsMissingDocuments() async {
        let service = MockFolioService(documents: [pdf("Gas bill")])
        await service.setFailure(.unreachable)
        let catalog = DocumentCatalog(service: service, locator: DocumentLocator())

        await #expect(throws: FolioError.unreachable) { try await catalog.transferFile(for: UUID()) }
    }

    @Test func recognizedTextComesFromTheFullDocument() async throws {
        let target = pdf("Gas bill", content: "Amount due: 42 EUR", ocr: .done)
        let catalog = DocumentCatalog(service: MockFolioService(documents: [target]), locator: DocumentLocator())

        #expect(try await catalog.recognizedText(for: target.id) == "Amount due: 42 EUR")
    }

    @Test func finishedScanWithoutTextHasNoTextToShare() async {
        let blank = pdf("Blank page", content: "", ocr: .done)
        let catalog = DocumentCatalog(service: MockFolioService(documents: [blank]), locator: DocumentLocator())

        await #expect(throws: ShortcutFailure.noRecognizedText) { try await catalog.recognizedText(for: blank.id) }
        #expect(!ShortcutError(mapping: ShortcutFailure.noRecognizedText).message.contains("yet"))
    }

    @Test func documentWithoutRecognizedTextHasNoTextToShare() async {
        let target = pdf("Fresh", content: "  \n")
        let catalog = DocumentCatalog(service: MockFolioService(documents: [target]), locator: DocumentLocator())

        await #expect(throws: ShortcutFailure.noRecognizedText) { try await catalog.recognizedText(for: target.id) }
    }

    @Test func entitiesResolveByIDEvenBeyondTheFirstPageAndFromTheirVault() async throws {
        let target = pdf("Lease")
        let vaultService = MockFolioService(documents: [target])
        let service = MockFolioService(documents: [], vaultServices: [otherVault: vaultService])
        let locator = DocumentLocator()
        locator.reset(server: server)
        locator.record(documentID: target.id, vaultID: otherVault)

        let found = try await DocumentCatalog(service: service, locator: locator).documents(ids: [target.id, UUID()])

        #expect(found.map(\.id) == [target.id])
    }
}
