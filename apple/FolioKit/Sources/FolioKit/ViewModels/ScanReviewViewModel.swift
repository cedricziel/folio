import CoreGraphics
import Foundation
import Observation

/// "Review a scan": shows the captured pages and files them as one PDF.
@MainActor @Observable
public final class ScanReviewViewModel {
    public enum State: Equatable {
        case idle
        case filing
        case filed
        case failed(String)
    }

    public let pages: [CGImage]
    public var title: String
    public private(set) var state = State.idle

    private let service: any FolioService
    private let defaultTitle: String
    public let destination: VaultDestinationViewModel?

    public init(
        pages: [CGImage], service: any FolioService, now: Date = Date(),
        destination: VaultDestinationViewModel? = nil
    ) {
        self.pages = pages
        self.service = service
        self.destination = destination
        defaultTitle = "Scan \(FolioFormat.isoDay(now)) \(Self.time(now))"
        title = defaultTitle
    }

    public func fileIt() async {
        guard state != .filing else { return }
        guard destination?.canUpload != false else {
            state = .failed("Choose a vault you can upload to.")
            return
        }
        let service = destination?.service ?? service
        state = .filing
        let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = name.isEmpty ? defaultTitle : name
        do {
            let pdf = try PDFBuilder.makePDF(from: pages)
            let file = UploadFile(
                data: pdf, filename: "\(title).pdf", mimeType: "application/pdf", title: title
            )
            _ = try await service.upload(file)
            state = .filed
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private static func time(_ date: Date) -> String {
        date.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
    }
}
