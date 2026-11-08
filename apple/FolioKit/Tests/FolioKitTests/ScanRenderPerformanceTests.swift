import CoreGraphics
import XCTest

@testable import FolioKit

/// Reports wall time, CPU and peak memory for rendering a representative multipage scan,
/// so regressions show up in the CI log. It sets no baseline, so it never fails on speed alone.
final class ScanRenderPerformanceTests: XCTestCase {
    /// Eight pages at roughly the resolution VisionKit returns for a letter-sized document.
    private let scan: [CGImage] = (0..<8).map { _ in
        let context = CGContext(
            data: nil, width: 1700, height: 2200, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(CGColor(red: 0.95, green: 0.95, blue: 0.9, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 1700, height: 2200))
        context.setFillColor(CGColor(red: 0.1, green: 0.1, blue: 0.1, alpha: 1))
        for line in stride(from: 200, to: 2000, by: 40) {
            context.fill(CGRect(x: 150, y: line, width: 1400, height: 12))
        }
        return context.makeImage()!
    }

    func testRenderingAnEightPageScan() {
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        var rendered: Data?
        measure(metrics: [XCTClockMetric(), XCTCPUMetric(), XCTMemoryMetric()], options: options) {
            rendered = try? PDFBuilder.makePDF(from: scan)
        }
        XCTAssertNotNil(rendered)
    }
}
