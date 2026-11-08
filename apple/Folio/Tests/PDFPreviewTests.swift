import CoreGraphics
import Foundation
import PDFKit
import XCTest

@MainActor
final class PDFPreviewTests: XCTestCase {
    private func threePages() throws -> PDFDocument {
        let data = NSMutableData()
        var box = CGRect(x: 0, y: 0, width: 612, height: 792)
        let consumer = try XCTUnwrap(CGDataConsumer(data: data as CFMutableData))
        let context = try XCTUnwrap(CGContext(consumer: consumer, mediaBox: &box, nil))
        for _ in 0..<3 {
            context.beginPDFPage(nil)
            context.endPDFPage()
        }
        context.closePDF()
        return try XCTUnwrap(PDFDocument(data: data as Data))
    }

    private func position(reporting pages: @escaping (Int) -> Void = { _ in }) -> PDFReadingPosition {
        let view = ReadingPDFView(frame: CGRect(x: 0, y: 0, width: 300, height: 200))
        return PDFReadingPosition(view: view, onPageChange: pages)
    }

    func testOpensAtTheRequestedPage() throws {
        let reader = position()
        reader.load(try threePages(), initialPage: 2)
        XCTAssertEqual(reader.currentPage, 2)
    }

    func testClampsAPageBeyondTheDocument() throws {
        let reader = position()
        reader.load(try threePages(), initialPage: 9)
        XCTAssertEqual(reader.currentPage, 3)
    }

    func testStartsAtTheFirstPageWithoutARequest() throws {
        let reader = position()
        reader.load(try threePages(), initialPage: nil)
        XCTAssertEqual(reader.currentPage, 1)
    }

    func testWaitsForLayoutBeforeMovingToThePage() throws {
        let reader = PDFReadingPosition(view: ReadingPDFView(frame: .zero), onPageChange: { _ in })
        reader.load(try threePages(), initialPage: 2)

        reader.view.frame = CGRect(x: 0, y: 0, width: 300, height: 200)
        reader.view.layout()

        XCTAssertEqual(reader.currentPage, 2)
    }

    func testALaterRequestForTheOpenDocumentMovesToItsPage() throws {
        let reader = position()
        reader.load(try threePages(), initialPage: 1)

        reader.show(page: 3)

        XCTAssertEqual(reader.currentPage, 3)
    }

    func testPublishesPageChanges() async throws {
        var reported: [Int] = []
        let changed = expectation(description: "page 3 reported")
        let reader = position {
            reported.append($0)
            if $0 == 3 { changed.fulfill() }
        }
        let document = try threePages()
        reader.load(document, initialPage: 1)

        reader.view.go(to: try XCTUnwrap(document.page(at: 2)))

        await fulfillment(of: [changed], timeout: 2)
        XCTAssertEqual(reported.last, 3)
    }
}
