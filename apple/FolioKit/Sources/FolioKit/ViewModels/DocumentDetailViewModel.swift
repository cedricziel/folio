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

    public let discussion: DocumentDiscussionViewModel
    public let document: Document
    public private(set) var preview = Preview.loading
    public let metadata: [MetadataField]
    /// The document as Spotlight and Shortcuts know it.
    public let summary: DocumentSummary

    private let service: any FolioService
    private var loading = false

    public init(
        document: Document, correspondent: String?, type: String?, service: any FolioService
    ) {
        discussion = DocumentDiscussionViewModel(documentID: document.id, service: service)
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
        guard !loading else { return }
        if case .ready = preview { return }
        loading = true
        defer { loading = false }
        preview = .loading
        // Navigation may cancel the view task while this model still owns the visible preview.
        let download = Task { [service, document] in
            try await service.downloadFile(for: document)
        }
        do {
            preview = .ready(try await download.value)
        } catch {
            preview = .failed(error.localizedDescription)
        }
    }
}
