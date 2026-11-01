import CoreGraphics
import Foundation

public enum PDFBuilder {
    public enum Failure: Error, Equatable {
        case noPages
        case renderingFailed
    }

    private static let a4Width: CGFloat = 595

    /// One page per image, A4 wide at the image's own aspect ratio so scans are not cropped.
    public static func makePDF(from images: [CGImage]) throws -> Data {
        guard !images.isEmpty else { throw Failure.noPages }
        let data = NSMutableData()
        guard let consumer = CGDataConsumer(data: data as CFMutableData),
              let context = CGContext(consumer: consumer, mediaBox: nil, nil)
        else { throw Failure.renderingFailed }

        for image in images {
            let height = (a4Width * CGFloat(image.height) / CGFloat(image.width)).rounded()
            var box = CGRect(x: 0, y: 0, width: a4Width, height: height)
            context.beginPage(mediaBox: &box)
            context.draw(image, in: box)
            context.endPage()
        }
        context.closePDF()
        return data as Data
    }
}
