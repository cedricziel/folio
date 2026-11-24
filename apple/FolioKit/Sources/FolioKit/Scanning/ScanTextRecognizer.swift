import CoreGraphics
import Foundation
import Vision

public enum ScanTextRecognizer {
    public enum Failure: Error {
        case cannotPrepareImage
    }

    private static let maximumDimension = 2_048
    private static let maximumTranscriptBytes = 16_384

    /// Reads one upright scan page on device. Returns a bounded transcript, or an empty string if
    /// Vision finds no text. Throws on image preparation, recognition failure, or cancellation.
    @concurrent
    public static func recognize(_ image: CGImage) async throws -> String {
        try Task.checkCancellation()
        let input = try recognitionImage(from: image)
        try Task.checkCancellation()
        var request = RecognizeDocumentsRequest()
        request.barcodeDetectionOptions.enabled = false
        request.textRecognitionOptions.automaticallyDetectLanguage = true

        let observations = try await request.perform(on: input)
        try Task.checkCancellation()
        let transcript = observations.first?.document.text.transcript ?? ""
        let utf8 = transcript.utf8
        var bytes = utf8.prefix(maximumTranscriptBytes)
        while bytes.endIndex != utf8.endIndex, utf8[bytes.endIndex] & 0xC0 == 0x80 {
            bytes = bytes.dropLast()
        }
        return String(decoding: bytes, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func recognitionImage(from image: CGImage) throws -> CGImage {
        let longestEdge = max(image.width, image.height)
        guard longestEdge > maximumDimension else { return image }

        let scale = CGFloat(maximumDimension) / CGFloat(longestEdge)
        let width = max(1, Int(CGFloat(image.width) * scale))
        let height = max(1, Int(CGFloat(image.height) * scale))
        guard let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
        ) else { throw Failure.cannotPrepareImage }

        context.interpolationQuality = .high
        context.setFillColor(gray: 1, alpha: 1)
        let bounds = CGRect(x: 0, y: 0, width: width, height: height)
        context.fill(bounds)
        context.draw(image, in: bounds)
        guard let resized = context.makeImage() else { throw Failure.cannotPrepareImage }
        return resized
    }
}
