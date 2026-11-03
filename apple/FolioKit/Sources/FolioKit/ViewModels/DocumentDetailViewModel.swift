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
    public private(set) var document: Document
    public private(set) var preview = Preview.loading
    public private(set) var metadata: [MetadataField]
    public private(set) var metadataFailure: String?
    public let history: [MetadataField]
    /// The document as Spotlight and Shortcuts know it.
    public let summary: DocumentSummary

    private let service: any FolioService
    private var loading = false
    private var metadataWrites = 0

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
        var history = [
            MetadataField(label: "Created", value: document.createdAt.formatted(), isRecord: true),
            MetadataField(label: "Created by", value: document.createdBy ?? "System or unknown", isRecord: false),
            MetadataField(label: "Updated", value: document.updatedAt.formatted(), isRecord: true),
            MetadataField(label: "Updated by", value: document.updatedBy ?? "System or unknown", isRecord: false),
        ]
        if let deletedAt = document.deletedAt {
            history.append(MetadataField(label: "Deleted", value: deletedAt.formatted(), isRecord: true))
        }
        self.history = history
    }

    public func saveDocumentDate(_ date: DocumentDate?) async throws {
        let updated = try await service.setDocumentDate(date, for: document.id)
        metadataWrites += 1
        applyMetadata(updated)
    }

    public func refreshMetadata() async {
        let writes = metadataWrites
        do {
            let updated = try await service.document(id: document.id)
            guard writes == metadataWrites else { return }
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
