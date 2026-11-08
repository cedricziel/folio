import CoreGraphics
import Foundation
import os

public enum PDFBuilder {
    public enum Failure: Error, Equatable {
        case noPages
        case renderingFailed
    }

    private static let a4Width: CGFloat = 595
    private static let signposter = OSSignposter(subsystem: "com.cedricziel.folio", category: "scan")
    private static let logger = Logger(subsystem: "com.cedricziel.folio", category: "scan")

    /// One page per image, A4 wide at the image's own aspect ratio so scans are not cropped.
    /// Throws `CancellationError` between pages once the calling task is cancelled.
    public static func makePDF(from images: [CGImage]) throws -> Data {
        guard !images.isEmpty else { throw Failure.noPages }
        let signpost = signposter.beginInterval(
            "Render scan PDF", id: signposter.makeSignpostID(), "\(images.count) pages"
        )
        defer { signposter.endInterval("Render scan PDF", signpost) }

        let data = NSMutableData()
        guard let consumer = CGDataConsumer(data: data as CFMutableData),
            let context = CGContext(consumer: consumer, mediaBox: nil, nil)
        else { throw Failure.renderingFailed }

        for image in images {
            try Task.checkCancellation()
            let height = (a4Width * CGFloat(image.height) / CGFloat(image.width)).rounded()
            var box = CGRect(x: 0, y: 0, width: a4Width, height: height)
            context.beginPage(mediaBox: &box)
            context.draw(image, in: box)
            context.endPage()
        }
        context.closePDF()
        return data as Data
    }

    /// Runs `makePDF` on a background thread so a multipage scan never blocks the UI.
    /// Cancelling the caller cancels the render, which stops before its next page.
    static func render(
        _ pages: ScanImages,
        using makePDF: @escaping @Sendable ([CGImage]) throws -> Data = PDFBuilder.makePDF(from:)
    ) async throws -> Data {
        let clock = ContinuousClock()
        let start = clock.now
        let work = Task.detached(priority: .userInitiated) { try makePDF(pages.images) }
        let data = try await withTaskCancellationHandler {
            try await work.value
        } onCancel: {
            work.cancel()
        }
        let milliseconds = Int(start.duration(to: clock.now) / .milliseconds(1))
        logger.info("Rendered \(pages.images.count) scan pages into \(data.count) bytes in \(milliseconds) ms")
        return data
    }
}

/// Scan pages handed to a background render. `CGImage` is immutable, so sharing it across threads is safe.
struct ScanImages: @unchecked Sendable {
    let images: [CGImage]
}
