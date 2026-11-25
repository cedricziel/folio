import PDFKit
import PaperKit
import PencilKit

@available(iOS 27.2, macOS 27.2, *)
@MainActor
final class PDFInkDraft {
    private let source: Data
    private let document: PDFDocument
    private var markupByPage: [Int: PaperMarkup] = [:]
    private var active: (index: Int, editor: PaperMarkupViewController)?

    var pageCount: Int { document.pageCount }

    init(data: Data) throws {
        var restrictedEntry: CGPDFObjectRef?
        guard let document = PDFDocument(data: data), !document.isEncrypted, document.allowsCommenting,
            document.pageCount > 0, let catalog = document.documentRef?.catalog,
            CGPDFDictionaryGetObject(catalog, "AcroForm", &restrictedEntry) == false,
            CGPDFDictionaryGetObject(catalog, "Perms", &restrictedEntry) == false
        else { throw DraftError.unsupportedPDF }
        for index in 0..<document.pageCount {
            guard let page = document.page(at: index),
                !page.annotations.contains(where: { $0.type == "Widget" })
            else { throw DraftError.unsupportedPDF }
        }
        source = data
        self.document = document
    }

    func editor(for index: Int) -> PaperMarkupViewController? {
        if let active, active.index == index { return active.editor }
        if let active {
            markupByPage[active.index] = active.editor.markup
            self.active = nil
        }
        guard let page = document.page(at: index), let canvas = Self.canvas(for: page) else { return nil }
        var features = FeatureSet.empty
        features.insert(.drawing)
        features.inks = [.pen]
        let markup = markupByPage.removeValue(forKey: index) ?? PaperMarkup(bounds: canvas)
        let editor = PaperMarkupViewController(markup: markup, supportedFeatureSet: features)
        editor.drawingTool = PKInkingTool(.pen, color: .black, width: 3)
        editor.indirectPointerTouchMode = .drawing
        let scale = min(1, 1600 / max(canvas.width, canvas.height))
        let thumbnailSize = CGSize(width: canvas.width * scale, height: canvas.height * scale)
        let thumbnail = page.thumbnail(of: thumbnailSize, for: .cropBox)
        #if os(iOS)
            editor.directTouchMode = .drawing
            let background = UIImageView(image: thumbnail)
            background.frame = canvas
            background.contentMode = .scaleToFill
        #else
            let background = NSImageView(frame: canvas)
            background.image = thumbnail
            background.imageScaling = .scaleAxesIndependently
        #endif
        editor.contentView = background
        active = (index, editor)
        return editor
    }

    func export() throws -> Data {
        guard let output = PDFDocument(data: source) else { throw DraftError.unsupportedPDF }
        var changed = false
        var markups = markupByPage
        if let active { markups[active.index] = active.editor.markup }
        for (index, markup) in markups {
            let strokes = markup.subelements.strokes
            guard !strokes.isEmpty else { continue }
            guard let page = output.page(at: index), let reference = page.pageRef else {
                throw DraftError.unsupportedPDF
            }
            let crop = page.bounds(for: .cropBox)
            let inverse = reference.getDrawingTransform(
                .cropBox, rect: markup.bounds, rotate: 0, preserveAspectRatio: true
            ).inverted()
            for stroke in strokes {
                let points = stroke.path.map { point in
                    let location = point.location.applying(stroke.transform)
                    let flipped = CGPoint(x: location.x, y: markup.bounds.height - location.y).applying(inverse)
                    return CGPoint(x: flipped.x - crop.minX, y: flipped.y - crop.minY)
                }
                guard let first = points.first else { continue }
                let annotation = PDFAnnotation(bounds: crop, forType: .ink, withProperties: nil)
                annotation.color = .black
                let border = PDFBorder()
                border.lineWidth = 3
                annotation.border = border
                #if os(iOS)
                    let path = UIBezierPath()
                    path.move(to: first)
                    for point in points.dropFirst() { path.addLine(to: point) }
                #else
                    let path = NSBezierPath()
                    path.move(to: first)
                    for point in points.dropFirst() { path.line(to: point) }
                #endif
                annotation.add(path)
                page.addAnnotation(annotation)
                changed = true
            }
        }
        guard changed else { throw DraftError.empty }
        guard let bytes = output.dataRepresentation() else { throw DraftError.unsupportedPDF }
        return bytes
    }

    private static func canvas(for page: PDFPage) -> CGRect? {
        let box = page.bounds(for: .cropBox)
        guard !box.isEmpty, box.width.isFinite, box.height.isFinite else { return nil }
        return CGRect(
            origin: .zero,
            size: page.rotation % 180 == 0
                ? box.size
                : CGSize(width: box.height, height: box.width))
    }

    private enum DraftError: LocalizedError {
        case unsupportedPDF
        case empty

        var errorDescription: String? {
            switch self {
            case .unsupportedPDF:
                "Folio can’t edit this PDF safely. Forms, signed PDFs, and protected PDFs are read-only."
            case .empty: "Draw a mark or sign a page before saving."
            }
        }
    }
}
