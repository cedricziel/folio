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

/// `Thread.isMainThread` is unavailable in async closures, so render closures ask through this.
private func isOnMainThread() -> Bool {
    Thread.isMainThread
}

@Suite(.timeLimit(.minutes(1))) struct PDFBuilderTests {
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

    @Test func stopsRenderingOnceCancelled() async {
        let pages = ScanImages(images: [page(width: 10, height: 10)])
        let render = Task.detached { () throws -> Data in
            while !Task.isCancelled { await Task.yield() }
            return try PDFBuilder.makePDF(from: pages.images)
        }
        render.cancel()
        await #expect(throws: CancellationError.self) { try await render.value }
    }

    @Test func cancellingTheCallerCancelsTheBackgroundRender() async {
        let (started, starter) = AsyncStream<Void>.makeStream()
        let rendering = Task {
            try await PDFBuilder.render(ScanImages(images: [page(width: 10, height: 10)])) { images in
                starter.yield()
                try await Task.sleep(for: .seconds(30))
                return try PDFBuilder.makePDF(from: images)
            }
        }
        var renderStarted = started.makeAsyncIterator()
        _ = await renderStarted.next()

        rendering.cancel()

        await #expect(throws: CancellationError.self) { try await rendering.value }
    }

    @Test func backgroundRenderMatchesTheDirectRenderPageForPage() async throws {
        let images = [page(width: 600, height: 800), page(width: 800, height: 600), page(width: 500, height: 1500)]
        let direct = try #require(PDFDocument(data: try PDFBuilder.makePDF(from: images)))
        let background = try #require(PDFDocument(data: try await PDFBuilder.render(ScanImages(images: images))))

        #expect(background.pageCount == direct.pageCount)
        for index in 0..<direct.pageCount {
            #expect(
                background.page(at: index)?.bounds(for: .mediaBox) == direct.page(at: index)?.bounds(for: .mediaBox)
            )
        }
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

@Suite(.timeLimit(.minutes(1))) @MainActor struct ScanReviewViewModelTests {
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

    @Test func rendersThePDFOffTheMainThread() async {
        let (renderedOnMain, recorder) = AsyncStream<Bool>.makeStream()
        let model = ScanReviewViewModel(
            pages: [page(width: 10, height: 10)], service: MockFolioService(),
            renderPDF: { images in
                recorder.yield(isOnMainThread())
                recorder.finish()
                return try PDFBuilder.makePDF(from: images)
            }
        )

        await model.fileIt()

        var recorded = renderedOnMain.makeAsyncIterator()
        #expect(await recorded.next() == false)
        #expect(model.state == .filed)
    }

    /// A render on the main actor would stall it for the whole render; off it, the main actor keeps ticking.
    @Test func mainActorStaysResponsiveWhileRenderingAScan() async throws {
        let (renderTimes, recorder) = AsyncStream<Duration>.makeStream()
        let model = ScanReviewViewModel(
            pages: letterScan(pages: 4), service: MockFolioService(),
            renderPDF: { images in
                // Renders until at least half a second has passed, so the hitch budget below stays well above
                // scheduling noise even on a fast machine.
                let start = ContinuousClock.now
                var pdf = try PDFBuilder.makePDF(from: images)
                while start.duration(to: .now) < .milliseconds(500) {
                    pdf = try PDFBuilder.makePDF(from: images)
                }
                recorder.yield(start.duration(to: .now))
                recorder.finish()
                return pdf
            }
        )
        let filing = Task { await model.fileIt() }
        let clock = ContinuousClock()
        let tick = Duration.milliseconds(10)
        var worstHitch = Duration.zero
        var ticks = 0
        repeat {
            let start = clock.now
            try await Task.sleep(for: tick)
            worstHitch = max(worstHitch, start.duration(to: clock.now) - tick)
            ticks += 1
        } while model.state == .filing
        await filing.value
        var recorded = renderTimes.makeAsyncIterator()
        let render = try #require(await recorded.next())

        reportMeasurement(
            "Main actor during a scan render of \(render): \(ticks) ticks, worst hitch \(worstHitch)"
        )
        #expect(model.state == .filed)
        #expect(worstHitch < render / 2)
    }

    @Test func uploadsEveryPageInScanOrder() async throws {
        let service = MockFolioService()
        let model = ScanReviewViewModel(
            pages: [page(width: 600, height: 800), page(width: 800, height: 600), page(width: 500, height: 1500)],
            service: service
        )

        await model.fileIt()

        let upload = try #require(await service.uploads.first)
        let pdf = try #require(PDFDocument(data: upload.data))
        let heights = (0..<pdf.pageCount).compactMap { pdf.page(at: $0)?.bounds(for: .mediaBox).height }
        #expect(heights == [793, 446, 1785])
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

@MainActor private final class Inbox {
    var showing: ScannedPages?
    var finished = 0
}

@Suite(.timeLimit(.minutes(1))) @MainActor struct ScanCompletionTests {
    private func startFilingA(on service: MockFolioService, inbox: Inbox) async -> ScanReviewViewModel {
        let scanA = ScannedPages(images: [page(width: 10, height: 10)])
        inbox.showing = scanA
        await service.suspendUploads()
        let model = ScanReviewViewModel(pages: scanA.images, service: service)
        model.file {
            inbox.finished += 1
            inbox.showing.close(scanA.id)
        }
        await service.waitForUploadRequest()
        return model
    }

    @Test func delayedSuccessDoesNotCloseALaterScan() async {
        let service = MockFolioService()
        let inbox = Inbox()
        let modelA = await startFilingA(on: service, inbox: inbox)
        let scanB = ScannedPages(images: [page(width: 10, height: 10)])
        inbox.showing = scanB

        await service.releaseUploads()
        await modelA.filing?.value

        #expect(modelA.state == .filed)
        #expect(inbox.finished == 1)
        #expect(inbox.showing?.id == scanB.id)
    }

    @Test func delayedFailureDoesNotCloseALaterScan() async {
        let service = MockFolioService()
        let inbox = Inbox()
        let modelA = await startFilingA(on: service, inbox: inbox)
        let scanB = ScannedPages(images: [page(width: 10, height: 10)])
        inbox.showing = scanB
        await service.setFailure(.unreachable)

        await service.releaseUploads()
        await modelA.filing?.value

        #expect(modelA.state == .failed("Couldn't reach the server."))
        #expect(inbox.finished == 0)
        #expect(inbox.showing?.id == scanB.id)
    }

    @Test func filedScanClosesItsOwnReview() async {
        let service = MockFolioService()
        let inbox = Inbox()
        let model = await startFilingA(on: service, inbox: inbox)

        await service.releaseUploads()
        await model.filing?.value

        #expect(inbox.finished == 1)
        #expect(inbox.showing == nil)
    }

    @Test func cancelledFilingNeverReportsCompletion() async {
        let service = MockFolioService()
        let inbox = Inbox()
        let model = await startFilingA(on: service, inbox: inbox)
        let filing = model.filing

        model.cancelFiling()
        await service.releaseUploads()
        await filing?.value

        #expect(inbox.finished == 0)
        #expect(inbox.showing != nil)
    }

    @Test func cancellingWhileRenderingNeverUploads() async {
        let service = MockFolioService()
        let inbox = Inbox()
        let (started, starter) = AsyncStream<Void>.makeStream()
        let (release, releaser) = AsyncStream<Void>.makeStream()
        let model = ScanReviewViewModel(
            pages: [page(width: 10, height: 10)], service: service,
            renderPDF: { images in
                starter.yield()
                for await _ in release { break }
                return try PDFBuilder.makePDF(from: images)
            }
        )
        model.file { inbox.finished += 1 }
        let filing = model.filing
        var renderStarted = started.makeAsyncIterator()
        _ = await renderStarted.next()

        model.cancelFiling()
        releaser.yield()
        await filing?.value

        #expect(await service.uploads.isEmpty)
        #expect(inbox.finished == 0)
        #expect(model.state == .idle)
    }
}
