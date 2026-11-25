import CoreText
import PDFKit
import PaperKit
import PencilKit
import XCTest

@MainActor
final class PDFInkDraftTests: XCTestCase {
    func testExportsInkAtRotatedCropCoordinatesWhileRetainingPageContent() throws {
        let source = try fixture()
        let draft = try PDFInkDraft(data: source)
        let editor = try XCTUnwrap(draft.editor(for: 0))
        var markup = try XCTUnwrap(editor.markup)
        let points = [CGPoint(x: 30, y: 50), CGPoint(x: 48, y: 70)]
        let strokePoints = points.enumerated().map { offset, point in
            PKStrokePoint(
                location: point, timeOffset: Double(offset), size: CGSize(width: 3, height: 3),
                opacity: 1, force: 1, azimuth: 0, altitude: .pi / 2
            )
        }
        markup.append(
            contentsOf: PKDrawing(strokes: [
                PKStroke(
                    ink: PKInk(.pen, color: .black),
                    path: PKStrokePath(controlPoints: strokePoints, creationDate: Date()))
            ]))
        editor.markup = markup

        let result = try XCTUnwrap(PDFDocument(data: try draft.export()))
        let page = try XCTUnwrap(result.page(at: 0))
        XCTAssertEqual(result.pageCount, 2)
        XCTAssertEqual(page.rotation, 90)
        XCTAssertEqual(page.bounds(for: .cropBox), CGRect(x: 17, y: 29, width: 200, height: 280))
        XCTAssertTrue(page.string?.contains("Folio fixture") == true)
        XCTAssertTrue(page.annotations.contains { $0.contents == "Existing note" })
        let ink = try XCTUnwrap(page.annotations.first { $0.paths?.count == 1 })
        let path = try XCTUnwrap(ink.paths?.first)
        var first = [CGPoint](repeating: .zero, count: 3)
        XCTAssertEqual(path.element(at: 0, associatedPoints: &first), .moveTo)
        XCTAssertEqual(first[0].x, 50, accuracy: 0.01)
        XCTAssertEqual(first[0].y, 30, accuracy: 0.01)
        XCTAssertEqual(PDFDocument(data: source)?.page(at: 0)?.annotations.count, 1)
    }

    func testCannotExportAnEmptyDraft() throws {
        let draft = try PDFInkDraft(data: fixture())
        XCTAssertNotNil(draft.editor(for: 0))
        XCTAssertThrowsError(try draft.export())
    }

    func testRejectsPDFWithSignatureWidget() throws {
        let document = try XCTUnwrap(PDFDocument(data: fixture()))
        let page = try XCTUnwrap(document.page(at: 0))
        let widget = PDFAnnotation(
            bounds: CGRect(x: 10, y: 10, width: 100, height: 20), forType: .widget, withProperties: nil
        )
        widget.widgetFieldType = .signature
        widget.fieldName = "Fixture signature"
        page.addAnnotation(widget)

        let bytes = try XCTUnwrap(document.dataRepresentation())
        XCTAssertThrowsError(try PDFInkDraft(data: bytes))
    }

    private func fixture() throws -> Data {
        let data = NSMutableData()
        var box = CGRect(x: 0, y: 0, width: 300, height: 400)
        let consumer = try XCTUnwrap(CGDataConsumer(data: data))
        let context = try XCTUnwrap(CGContext(consumer: consumer, mediaBox: &box, nil))
        context.beginPDFPage(nil)
        context.textPosition = CGPoint(x: 80, y: 80)
        let text = NSAttributedString(string: "Folio fixture", attributes: [.font: NSFont.systemFont(ofSize: 12)])
        CTLineDraw(CTLineCreateWithAttributedString(text), context)
        context.endPDFPage()
        context.beginPDFPage(nil)
        context.endPDFPage()
        context.closePDF()

        let document = try XCTUnwrap(PDFDocument(data: data as Data))
        let page = try XCTUnwrap(document.page(at: 0))
        page.setBounds(CGRect(x: 17, y: 29, width: 200, height: 280), for: .cropBox)
        page.rotation = 90
        let note = PDFAnnotation(
            bounds: CGRect(x: 100, y: 100, width: 20, height: 20), forType: .text, withProperties: nil
        )
        note.contents = "Existing note"
        page.addAnnotation(note)
        return try XCTUnwrap(document.dataRepresentation())
    }
}
