import CoreGraphics
import CoreText
import Foundation
import Vision

private func fixture(_ name: String, poor: Bool = false) -> CGImage {
    let height = name == "receipt" ? 1300 : 1600
    let width = name == "receipt" ? 680 : 1200
    let c = CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
    c.setFillColor(gray: poor ? 0.72 : 1, alpha: 1)
    c.fill(CGRect(x: 0, y: 0, width: width, height: height))
    if poor {
        c.translateBy(x: 105, y: -15)
        c.rotate(by: 0.08)
    }
    func line(_ s: String, _ x: Int, _ y: Int, _ size: CGFloat = 30) {
        c.textPosition = CGPoint(x: x, y: y)
        CTLineDraw(
            CTLineCreateWithAttributedString(
                NSAttributedString(
                    string: s,
                    attributes: [
                        NSAttributedString.Key(kCTFontAttributeName as String): CTFontCreateWithName(
                            "Helvetica" as CFString, size, nil),
                        NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(
                            gray: poor ? 0.56 : 0, alpha: 1),
                    ])), c)
    }
    if name == "receipt" {
        for (i, s) in [
            "NORTH MARKET", "24 November 2026", "Receipt 05173", "2 Apples                  1.80",
            "1 Oat milk                2.49", "Total EUR                 4.29",
        ].enumerated() {
            line(s, 65, 1160 - i * 52, 33)
        }
    } else if name == "table" {
        line("Equipment inventory", 100, 1450, 42)
        c.setStrokeColor(gray: 0, alpha: 1)
        c.setLineWidth(2)
        for x in [90, 460, 710, 1000] {
            c.move(to: CGPoint(x: x, y: 1040)); c.addLine(to: CGPoint(x: x, y: 1360))
        }
        for y in stride(from: 1040, through: 1360, by: 80) {
            c.move(to: CGPoint(x: 90, y: y)); c.addLine(to: CGPoint(x: 1000, y: y))
        }
        c.strokePath()
        for (r, row) in [
            ["Item", "Room", "Count"], ["Adapters", "Lab", "12"], ["Monitors", "Office", "4"],
            ["Keyboards", "Storage", "19"],
        ].enumerated() {
            for (i, cell) in row.enumerated() { line(cell, [120, 490, 740][i], 1310 - r * 80) }
        }
    } else {
        for (i, s) in [
            "Terminbestätigung", "24. November 2026", "Aktenzeichen AB-4719", "Sehr geehrte Frau Muster,",
            "Ihr Termin findet am 4. Dezember statt.", "Bitte bringen Sie Ihren Ausweis mit.",
        ].enumerated() {
            line(s, 115, 1440 - i * 64, i == 0 ? 45 : poor ? 21 : 32)
        }
    }
    return c.makeImage()!
}

@main struct Probe {
    static func main() async throws {
        guard CommandLine.arguments.count == 2,
            let name = CommandLine.arguments.last,
            ["letter", "receipt", "table", "poor"].contains(name)
        else {
            print("Usage: scan-recognition-probe letter|receipt|table|poor")
            exit(EXIT_FAILURE)
        }
        var request = RecognizeDocumentsRequest()
        request.barcodeDetectionOptions.enabled = false
        request.textRecognitionOptions.automaticallyDetectLanguage = true
        let image = fixture(name, poor: name == "poor")
        let clock = ContinuousClock()
        print("languages:", request.supportedRecognitionLanguages.map { $0.minimalIdentifier }.joined(separator: ","))
        for _ in 0..<3 {
            let start = clock.now
            let observations = try await request.perform(on: image)
            print("time:", start.duration(to: clock.now))
            for observation in observations {
                let d = observation.document
                print(
                    "text:", d.text.transcript, "paragraphs:", d.paragraphs.count, "title:", d.title?.transcript ?? "-",
                    "detected:", d.text.detectedData.count)
                for table in d.tables {
                    print("table:", table.rows.map { $0.map { $0.content.text.transcript } })
                }
            }
        }
    }
}
