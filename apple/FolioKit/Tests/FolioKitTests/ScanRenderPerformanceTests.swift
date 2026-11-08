import CoreGraphics
import Foundation
import PDFKit
import Testing
import XCTest

@testable import FolioKit

/// Pages at roughly the resolution VisionKit returns for a letter-sized document.
func letterScan(pages: Int) -> [CGImage] {
    (0..<pages).map { _ in
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
}

/// Prints a measurement and, on GitHub Actions, adds it to the job summary so it can be read without the full log.
func reportMeasurement(_ line: String) {
    print(line)
    guard let path = ProcessInfo.processInfo.environment["GITHUB_STEP_SUMMARY"],
        let summary = FileHandle(forWritingAtPath: path)
    else { return }
    defer { try? summary.close() }
    _ = try? summary.seekToEnd()
    try? summary.write(contentsOf: Data("- \(line)\n".utf8))
}

/// Reports wall time, CPU and peak memory for rendering a representative multipage scan,
/// so regressions show up in the CI log. It sets no baseline, so it never fails on speed alone.
final class ScanRenderPerformanceTests: XCTestCase {
    private let scan = letterScan(pages: 8)

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

struct ScanRenderCostTests {
    @Test func reportsTheCostOfRenderingAnEightPageScan() async throws {
        let scan = ScanImages(images: letterScan(pages: 8))
        let clock = ContinuousClock()
        let start = clock.now
        let data = try await PDFBuilder.render(scan)
        let milliseconds = Int(start.duration(to: clock.now) / .milliseconds(1))

        reportMeasurement("Scan render cost: 8 pages at 1700×2200, \(data.count / 1024) KiB PDF, \(milliseconds) ms")
        #expect(PDFDocument(data: data)?.pageCount == 8)
    }
}
