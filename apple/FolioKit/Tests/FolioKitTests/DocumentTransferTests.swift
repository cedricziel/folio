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

private func scratch() -> URL {
    let url = FileManager.default.temporaryDirectory.appending(
        path: "DocumentTransferTests-\(UUID().uuidString)", directoryHint: .isDirectory)
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

private func id(_ document: Document, vault: UUID? = nil) -> DocumentEntityID {
    DocumentEntityID(documentID: document.id, vaultID: vault)
}

/// A clock tests move forward by hand.
private final class TestClock: @unchecked Sendable {
    private let lock = NSLock()
    private var current = Date(timeIntervalSince1970: 1_000_000)
    var now: Date { lock.withLock { current } }
    func advance(_ seconds: TimeInterval) { lock.withLock { current += seconds } }
}

struct DocumentEntityIDTests {
    @Test func identifierCarriesTheVaultAndMatchesTheSpotlightLink() {
        let documentID = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!
        let scoped = DocumentEntityID(documentID: documentID, vaultID: otherVault)

        #expect(
            scoped.entityIdentifierString
                == DocumentLink(documentID: documentID, vaultID: otherVault).spotlightIdentifier)
        #expect(DocumentEntityID.entityIdentifier(for: scoped.entityIdentifierString) == scoped)
    }

    @Test func identifiersSavedBeforeVaultsStillResolveInTheDefaultVault() {
        let documentID = UUID()
        let legacy = DocumentEntityID.entityIdentifier(for: documentID.uuidString)

        #expect(legacy == DocumentEntityID(documentID: documentID))
        #expect(legacy?.entityIdentifierString == documentID.uuidString.lowercased())
        #expect(DocumentEntityID.entityIdentifier(for: "nonsense") == nil)
    }

    @Test func webLinkPointsAtTheDocumentInItsVault() {
        let documentID = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!
        let document = "documents/3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f"

        #expect(
            DocumentEntityID(documentID: documentID).webURL(server: server).absoluteString
                == "https://folio.example/\(document)")
        #expect(
            DocumentEntityID(documentID: documentID, vaultID: otherVault).webURL(server: server).absoluteString
                == "https://folio.example/vaults/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee/\(document)")
    }

    @Test func entityFromAVaultSummaryCarriesThatVault() {
        let document = pdf("Lease")
        let summary = DocumentSummary(document, correspondent: nil, documentType: nil, vaultID: otherVault)
        let entity = DocumentEntity(summary)

        #expect(entity.id == DocumentEntityID(documentID: document.id, vaultID: otherVault))
        #expect(entity.documentID == document.id)
    }
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

    @Test func eachContentTypeIsExportedUnderOneDeclaredType() {
        #expect(Set(DocumentTransfer.fileTypes).count == DocumentTransfer.fileTypes.count)
        #expect(DocumentTransfer.exportedFileType(for: .pdf) == .pdf)
        #expect(DocumentTransfer.exportedFileType(for: .png) == .png)
        #expect(DocumentTransfer.exportedFileType(for: .tiff) == .image)
        #expect(DocumentTransfer.exportedFileType(for: .plainText) == .content)
        #expect(DocumentTransfer.exportedFileType(for: .zip) == .data)
    }

    @Test func entityCarriesTheContentTypeAndWhetherTextIsAvailable() {
        let read = DocumentEntity(
            DocumentSummary(pdf("Gas bill", content: "Amount due", ocr: .done), correspondent: nil, documentType: nil))
        #expect(read.contentType == .pdf)
        #expect(read.hasRecognizedText)

        let blank = DocumentEntity(
            DocumentSummary(pdf("Blank", content: " \n", ocr: .done), correspondent: nil, documentType: nil))
        #expect(!blank.hasRecognizedText)

        let listRow = DocumentEntity(DocumentSummary(pdf("Row", ocr: .done), correspondent: nil, documentType: nil))
        #expect(!listRow.hasRecognizedText)

        let image = Document(title: "Receipt", originalFilename: "receipt.jpg", mimeType: "image/jpeg")
        #expect(DocumentEntity(DocumentSummary(image, correspondent: nil, documentType: nil)).contentType == .jpeg)
    }

    @Test func processingProgressAloneDoesNotChangeWhatSpotlightShows() {
        let pending = pdf("Gas bill")
        var done = pending
        done.ocrStatus = .done
        done.content = "Amount due"

        #expect(
            DocumentSummary(pending, correspondent: nil, documentType: nil).indexedFields
                == DocumentSummary(done, correspondent: nil, documentType: nil).indexedFields)
    }
}

struct DocumentTransferCatalogTests {
    @Test func exportedFileIsTheOriginalWithItsFilenameAndType() async throws {
        let target = pdf("Gas bill")
        let service = MockFolioService(documents: [target])
        let root = scratch()
        await service.writeDownloads(into: root)
        let cache = TransferFileCache(directory: root.appending(path: "cache"))

        let file = try await DocumentCatalog(service: service).transferFile(for: id(target), cache: cache)

        #expect(file.url.lastPathComponent == "Gas bill.pdf")
        #expect(file.contentType == .pdf)
        #expect(FileManager.default.fileExists(atPath: file.url.path))
        #expect(await service.downloads == 1)
    }

    @Test func exportDownloadsFromTheDocumentsOwnVault() async throws {
        let target = pdf("Lease")
        let vaultService = MockFolioService(documents: [target])
        let root = scratch()
        await vaultService.writeDownloads(into: root)
        let service = MockFolioService(documents: [], vaultServices: [otherVault: vaultService])
        let cache = TransferFileCache(directory: root.appending(path: "cache"))

        let file = try await DocumentCatalog(service: service).transferFile(
            for: id(target, vault: otherVault), cache: cache)

        #expect(file.url.lastPathComponent == "Lease.pdf")
        #expect(await vaultService.downloads == 1)
        #expect(await service.downloads == 0)
    }

    @Test func repeatedExportsReuseTheDownloadAndRemoveItsTemporaryFolder() async throws {
        let target = pdf("Gas bill")
        let service = MockFolioService(documents: [target])
        let root = scratch()
        await service.writeDownloads(into: root)
        let cache = TransferFileCache(directory: root.appending(path: "cache"))
        let catalog = DocumentCatalog(service: service)

        let first = try await catalog.transferFile(for: id(target), cache: cache)
        let second = try await catalog.transferFile(for: id(target), cache: cache)

        #expect(first.url == second.url)
        #expect(await service.downloads == 1)
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: root.path).filter { $0.hasPrefix("folio-") }
        #expect(leftovers.isEmpty)
    }

    @Test func exportReusesTheOpenPreviewWithoutDownloading() async throws {
        let target = pdf("Gas bill")
        let service = MockFolioService(documents: [target])
        let root = scratch()
        let preview = root.appending(path: "Gas bill.pdf")
        try Data("preview".utf8).write(to: preview)
        let cache = TransferFileCache(directory: root.appending(path: "cache"))
        cache.adopt(preview, for: id(target))

        let file = try await DocumentCatalog(service: service).transferFile(for: id(target), cache: cache)

        #expect(file.url == preview)
        #expect(file.contentType == .pdf)
        #expect(await service.downloads == 0)
    }

    @Test func exportOfAnInaccessibleOrTrashedDocumentFails() async {
        let trashed = Document(title: "Old", originalFilename: "old.pdf", deletedAt: Date(timeIntervalSince1970: 0))
        let catalog = DocumentCatalog(service: MockFolioService(documents: [trashed]))
        let cache = TransferFileCache(directory: scratch())

        await #expect(throws: ShortcutFailure.documentNotFound) {
            try await catalog.transferFile(for: DocumentEntityID(documentID: UUID()), cache: cache)
        }
        await #expect(throws: ShortcutFailure.documentNotFound) {
            try await catalog.transferFile(for: id(trashed), cache: cache)
        }
    }

    @Test func serverFailuresAreNotReportedAsMissingDocuments() async {
        let service = MockFolioService(documents: [pdf("Gas bill")])
        await service.setFailure(.unreachable)
        let cache = TransferFileCache(directory: scratch())

        await #expect(throws: FolioError.unreachable) {
            try await DocumentCatalog(service: service).transferFile(
                for: DocumentEntityID(documentID: UUID()), cache: cache)
        }
    }

    @Test func recognizedTextComesFromTheFullDocument() async throws {
        let target = pdf("Gas bill", content: "Amount due: 42 EUR", ocr: .done)
        let catalog = DocumentCatalog(service: MockFolioService(documents: [target]))

        #expect(try await catalog.recognizedText(for: id(target)) == "Amount due: 42 EUR")
    }

    @Test func documentWithoutRecognizedTextHasNoTextToShare() async {
        let target = pdf("Fresh", content: "  \n", ocr: .done)
        let catalog = DocumentCatalog(service: MockFolioService(documents: [target]))

        await #expect(throws: ShortcutFailure.noRecognizedText) { try await catalog.recognizedText(for: id(target)) }
        #expect(!ShortcutError(mapping: ShortcutFailure.noRecognizedText).message.contains("yet"))
    }
}

struct DocumentResolutionTests {
    @Test func entitiesResolveFromTheirOwnVaultInTheOrderAsked() async throws {
        let inDefault = pdf("Default")
        let inOther = pdf("Other vault")
        let vaultService = MockFolioService(documents: [inOther])
        let service = MockFolioService(documents: [inDefault], vaultServices: [otherVault: vaultService])

        let found = try await DocumentCatalog(service: service).documents(ids: [
            id(inOther, vault: otherVault), DocumentEntityID(documentID: UUID()), id(inDefault),
        ])

        #expect(found.map(\.id) == [inOther.id, inDefault.id])
        #expect(found.map(\.vaultID) == [otherVault, nil])
    }

    @Test func forbiddenAndMissingDocumentsAreMissingAndTheRestStillResolve() async throws {
        let kept = pdf("Kept")
        let forbidden = pdf("Forbidden")
        let forbiddenVault = UUID()
        let refusing = MockFolioService(documents: [forbidden])
        await refusing.setFailure(.server(status: 403, message: nil))
        let catalog = DocumentCatalog(
            service: MockFolioService(documents: [kept], vaultServices: [forbiddenVault: refusing]))

        let resolution = try await catalog.resolve(ids: [id(forbidden, vault: forbiddenVault), id(kept)])

        #expect(resolution.found.map(\.id) == [kept.id])
        #expect(resolution.missing == [id(forbidden, vault: forbiddenVault)])
        #expect(resolution.unresolved.isEmpty)
    }

    @Test func transientFailuresLeaveDocumentsUnresolvedWithoutFailingTheRest() async throws {
        let kept = pdf("Kept")
        let flakyVault = UUID()
        let flaky = MockFolioService(documents: [pdf("Elsewhere")])
        await flaky.setFailure(.unreachable)
        let catalog = DocumentCatalog(
            service: MockFolioService(documents: [kept], vaultServices: [flakyVault: flaky]))
        let elsewhere = DocumentEntityID(documentID: UUID(), vaultID: flakyVault)

        let resolution = try await catalog.resolve(ids: [elsewhere, id(kept)])

        #expect(resolution.found.map(\.id) == [kept.id])
        #expect(resolution.unresolved == [elsewhere])
    }

    @Test func aServerThatAnswersForNothingFailsTheLookup() async {
        let service = MockFolioService(documents: [pdf("Kept")])
        await service.setFailure(.unreachable)

        await #expect(throws: FolioError.unreachable) {
            try await DocumentCatalog(service: service).resolve(ids: [DocumentEntityID(documentID: UUID())])
        }
    }

    @Test func manyDocumentsResolveWithBoundedConcurrency() async throws {
        let documents = (0..<12).map { pdf("Doc \($0)") }
        let catalog = DocumentCatalog(service: MockFolioService(documents: documents))

        let found = try await catalog.documents(ids: documents.map { id($0) })

        #expect(found.map(\.id) == documents.map(\.id))
    }
}

struct TransferFileCacheTests {
    @Test func oldDownloadsAreDeletedButAdoptedPreviewsAreNot() throws {
        let root = scratch()
        let clock = TestClock()
        let cache = TransferFileCache(directory: root.appending(path: "cache"), maxAge: 60, now: { clock.now })
        let downloadFolder = root.appending(path: "folio-download", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: downloadFolder, withIntermediateDirectories: true)
        let download = downloadFolder.appending(path: "a.pdf")
        try Data("a".utf8).write(to: download)
        let preview = root.appending(path: "b.pdf")
        try Data("b".utf8).write(to: preview)
        let a = DocumentEntityID(documentID: UUID())
        let b = DocumentEntityID(documentID: UUID())

        let stored = try cache.store(download, for: a)
        cache.adopt(preview, for: b)
        #expect(!FileManager.default.fileExists(atPath: downloadFolder.path))

        clock.advance(61)
        cache.prune()

        #expect(!FileManager.default.fileExists(atPath: stored.path))
        #expect(cache.file(for: a) == nil)
        #expect(FileManager.default.fileExists(atPath: preview.path))
    }

    @Test func onlyTheNewestDownloadsAreKept() throws {
        let root = scratch()
        let clock = TestClock()
        let cache = TransferFileCache(directory: root.appending(path: "cache"), maxFiles: 2, now: { clock.now })
        var stored: [URL] = []
        for index in 0..<3 {
            let download = root.appending(path: "\(index).pdf")
            try Data("\(index)".utf8).write(to: download)
            stored.append(try cache.store(download, for: DocumentEntityID(documentID: UUID())))
            clock.advance(1)
        }

        #expect(!FileManager.default.fileExists(atPath: stored[0].path))
        #expect(FileManager.default.fileExists(atPath: stored[1].path))
        #expect(FileManager.default.fileExists(atPath: stored[2].path))
    }

    @Test func signOutAnotherServerOrAnInteractiveSignInDeleteEveryDownload() throws {
        let root = scratch()
        let cache = TransferFileCache(directory: root.appending(path: "cache"))
        func storeOne() throws -> URL {
            let download = root.appending(path: "\(UUID().uuidString).pdf")
            try Data("x".utf8).write(to: download)
            return try cache.store(download, for: DocumentEntityID(documentID: UUID()))
        }
        cache.sessionChanged(to: .signedIn(server))

        let kept = try storeOne()
        cache.sessionChanged(to: .interrupted(server))
        cache.sessionChanged(to: .signedIn(server))
        #expect(FileManager.default.fileExists(atPath: kept.path))

        cache.sessionChanged(to: .signedIn(URL(string: "https://other.folio.example")!))
        #expect(!FileManager.default.fileExists(atPath: kept.path))

        let beforeSignOut = try storeOne()
        cache.sessionChanged(to: .signedOut)
        #expect(!FileManager.default.fileExists(atPath: beforeSignOut.path))

        let beforeReSignIn = try storeOne()
        cache.signedIn(to: server)
        #expect(!FileManager.default.fileExists(atPath: beforeReSignIn.path))
    }
}
