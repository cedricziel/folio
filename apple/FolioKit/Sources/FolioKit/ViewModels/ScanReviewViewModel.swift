import CoreGraphics
import Foundation
import Observation

/// The pages of one scan waiting for review.
public struct ScannedPages: Identifiable {
    public let id = UUID()
    public let images: [CGImage]

    public init(images: [CGImage]) {
        self.images = images
    }
}

extension Optional where Wrapped: Identifiable {
    /// Clears the item only while `id` is still the one shown, so a late completion
    /// cannot close an item presented after it.
    public mutating func close(_ id: Wrapped.ID) {
        if self?.id == id { self = nil }
    }
}

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
    private let renderPDF: @Sendable ([CGImage]) async throws -> Data
    private(set) var filing: Task<Void, Never>?

    public init(
        pages: [CGImage], service: any FolioService, now: Date = Date(),
        destination: VaultDestinationViewModel? = nil,
        renderPDF: @escaping @Sendable ([CGImage]) async throws -> Data = PDFBuilder.makePDF(from:)
    ) {
        self.pages = pages
        self.service = service
        self.renderPDF = renderPDF
        self.destination = destination
        defaultTitle = "Scan \(FolioFormat.isoDay(now)) \(Self.time(now))"
        title = defaultTitle
    }

    /// Files the scan in a task this model owns. `onFiled` runs only if the upload
    /// succeeds and the filing was not cancelled.
    public func file(onFiled: @escaping @MainActor () -> Void) {
        guard filing == nil else { return }
        filing = Task {
            await fileIt()
            filing = nil
            if state == .filed, !Task.isCancelled { onFiled() }
        }
    }

    public func cancelFiling() {
        filing?.cancel()
        filing = nil
    }

    func fileIt() async {
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
            let pdf = try await PDFBuilder.render(ScanImages(images: pages), using: renderPDF)
            try Task.checkCancellation()
            let file = UploadFile(
                data: pdf, filename: "\(title).pdf", mimeType: "application/pdf", title: title
            )
            _ = try await service.upload(file)
            state = .filed
        } catch is CancellationError {
            state = .idle
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private static func time(_ date: Date) -> String {
        date.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
    }
}
