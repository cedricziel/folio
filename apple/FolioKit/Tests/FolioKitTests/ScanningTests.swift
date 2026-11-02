import CoreGraphics
import Foundation
import PDFKit
import Testing

@testable import FolioKit

private func page(width: Int, height: Int) -> CGImage {
    let context = CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(red: 0.9, green: 0.9, blue: 0.8, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    return context.makeImage()!
}

struct PDFBuilderTests {
    @Test func buildsOnePDFPagePerImage() throws {
        let data = try PDFBuilder.makePDF(from: [
            page(width: 600, height: 800), page(width: 800, height: 600),
        ])
        #expect(data.prefix(4) == Data("%PDF".utf8))
        let pdf = try #require(PDFDocument(data: data))
        #expect(pdf.pageCount == 2)
    }

    @Test func pagesKeepTheScanAspectRatioAtA4Width() throws {
        let data = try PDFBuilder.makePDF(from: [page(width: 1000, height: 1400)])
        let bounds = try #require(PDFDocument(data: data)?.page(at: 0)?.bounds(for: .mediaBox))
        #expect(abs(bounds.width - 595) < 1)
        #expect(abs(bounds.height - 833) < 1)
    }

    @Test func refusesAnEmptyScan() {
        #expect(throws: PDFBuilder.Failure.noPages) { try PDFBuilder.makePDF(from: []) }
    }
}

struct UploadFileTests {
    @Test func infersMimeTypeFromTheFileExtension() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appending(path: "Bill.pdf")
        try Data("%PDF".utf8).write(to: url)

        let file = try UploadFile(contentsOf: url)
        #expect(file.filename == "Bill.pdf")
        #expect(file.mimeType == "application/pdf")
        #expect(file.data == Data("%PDF".utf8))
    }

    @Test func fallsBackToOctetStream() {
        #expect(UploadFile.mimeType(forFilename: "blob.zzzunknown") == "application/octet-stream")
    }
}

@MainActor struct ScanReviewViewModelTests {
    @Test func scanUploadsToItsChosenVault() async {
        let vault = Vault(id: UUID(), name: "Household", role: .contributor)
        let target = MockFolioService()
        let source = MockFolioService(vaultServices: [vault.id: target])
        let destination = VaultDestinationViewModel(
            service: source, catalog: VaultCatalog(vaults: [vault], defaultVaultID: vault.id)
        )
        let model = ScanReviewViewModel(
            pages: [page(width: 10, height: 10)], service: source, destination: destination
        )

        await model.fileIt()

        #expect(await target.uploads.first?.mimeType == "application/pdf")
        #expect(await source.uploads.isEmpty)
        #expect(model.state == .filed)
    }

    @Test func viewerCannotFileAScan() async {
        let vault = Vault(id: UUID(), name: "Household", role: .viewer)
        let service = MockFolioService()
        let destination = VaultDestinationViewModel(
            service: service, catalog: VaultCatalog(vaults: [vault], defaultVaultID: vault.id)
        )
        let model = ScanReviewViewModel(
            pages: [page(width: 10, height: 10)], service: service, destination: destination
        )

        await model.fileIt()

        #expect(model.state == .failed("Choose a vault you can upload to."))
        #expect(await service.uploads.isEmpty)
    }

    @Test func fileItUploadsThePagesAsOnePDFWithTheTitle() async {
        let service = MockFolioService()
        let model = ScanReviewViewModel(
            pages: [page(width: 600, height: 800)], service: service,
            now: Date(timeIntervalSince1970: 1_741_950_000)
        )
        model.title = "Electricity bill"
        await model.fileIt()

        let upload = await service.uploads.first
        #expect(upload?.mimeType == "application/pdf")
        #expect(upload?.filename == "Electricity bill.pdf")
        #expect(upload?.title == "Electricity bill")
        #expect(model.state == .filed)
    }

    @Test func blankTitleFallsBackToADatedName() async {
        let service = MockFolioService()
        let model = ScanReviewViewModel(
            pages: [page(width: 10, height: 10)], service: service,
            now: Date(timeIntervalSince1970: 1_741_950_000)
        )
        model.title = "  "
        await model.fileIt()
        #expect(await service.uploads.first?.title?.hasPrefix("Scan 2025-03-14") == true)
    }

    @Test func uploadFailureKeepsThePagesForRetry() async {
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        let model = ScanReviewViewModel(pages: [page(width: 10, height: 10)], service: service)
        await model.fileIt()
        #expect(model.state == .failed("Couldn't reach the server."))
        #expect(model.pages.count == 1)
    }
}
