import Foundation
import Observation

public struct MetadataField: Identifiable, Equatable, Sendable {
    public let label: String
    public let value: String
    /// Values people compare across documents are set in the mono `meta` style.
    public let isRecord: Bool

    public var id: String {
        label
    }
}

@MainActor @Observable
public final class DocumentDetailViewModel {
    public enum Preview: Equatable {
        case loading
        case ready(URL)
        case failed(String)
    }

    public let document: Document
    public private(set) var preview = Preview.loading
    public let metadata: [MetadataField]
    /// The document as Spotlight and Shortcuts know it.
    public let summary: DocumentSummary

    private let service: any FolioService

    public init(
        document: Document, correspondent: String?, type: String?, service: any FolioService
    ) {
        self.document = document
        self.service = service
        summary = DocumentSummary(document, correspondent: correspondent, documentType: type)
        var fields = [
            MetadataField(label: "From", value: correspondent ?? "Not set", isRecord: false),
            MetadataField(label: "Type", value: type ?? "Not set", isRecord: false),
            MetadataField(label: "Date", value: FolioFormat.isoDay(document.createdAt), isRecord: true),
        ]
        if let pages = document.pageCount {
            fields.append(MetadataField(label: "Pages", value: String(pages), isRecord: true))
        }
        if let filename = document.originalFilename {
            fields.append(MetadataField(label: "File", value: filename, isRecord: true))
        }
        metadata = fields
    }

    public func loadPreview() async {
        preview = .loading
        do {
            preview = try .ready(await service.downloadFile(for: document))
        } catch {
            preview = .failed(error.localizedDescription)
        }
    }
}
