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

    public private(set) var document: Document
    public private(set) var preview = Preview.loading
    public private(set) var metadata: [MetadataField]
    public private(set) var metadataFailure: String?
    /// The document as Spotlight and Shortcuts know it.
    public let summary: DocumentSummary

    private let service: any FolioService
    private var loading = false

    public init(
        document: Document, correspondent: String?, type: String?, service: any FolioService
    ) {
        self.document = document
        self.service = service
        summary = DocumentSummary(document, correspondent: correspondent, documentType: type)
        var fields = [
            MetadataField(label: "From", value: correspondent ?? "Not set", isRecord: false),
            MetadataField(label: "Type", value: type ?? "Not set", isRecord: false),
            MetadataField(label: "Document date", value: document.documentDate?.rawValue ?? "Not set", isRecord: true),
            MetadataField(label: "Imported", value: FolioFormat.isoDay(document.createdAt), isRecord: true),
        ]
        if let pages = document.pageCount {
            fields.append(MetadataField(label: "Pages", value: String(pages), isRecord: true))
        }
        if let filename = document.originalFilename {
            fields.append(MetadataField(label: "File", value: filename, isRecord: true))
        }
        metadata = fields
    }

    public func saveDocumentDate(_ date: DocumentDate?) async throws {
        let updated = try await service.setDocumentDate(date, for: document.id)
        applyMetadata(updated)
    }

    public func refreshMetadata() async {
        do {
            let updated = try await service.document(id: document.id)
            applyMetadata(updated)
            metadataFailure = nil
        } catch is CancellationError {
        } catch {
            metadataFailure = error.localizedDescription
        }
    }

    private func applyMetadata(_ updated: Document) {
        document = updated
        if let index = metadata.firstIndex(where: { $0.label == "Document date" }) {
            metadata[index] = MetadataField(
                label: "Document date", value: updated.documentDate?.rawValue ?? "Not set", isRecord: true)
        }
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
