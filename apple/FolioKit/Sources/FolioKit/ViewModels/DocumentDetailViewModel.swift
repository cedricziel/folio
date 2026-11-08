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
    public private(set) var metadata: [MetadataField] = []
    public private(set) var metadataFailure: String?
    public private(set) var history: [MetadataField] = []
    public private(set) var savingReview = false
    public private(set) var reviewFailure: String?
    /// The document as Spotlight and Shortcuts know it.
    public private(set) var summary: DocumentSummary

    private let service: any FolioService
    private var correspondent: String?
    private var documentType: String?
    private var loading = false
    private var metadataWrites = 0

    public init(
        document: Document, correspondent: String?, type: String?, service: any FolioService
    ) {
        discussion = DocumentDiscussionViewModel(documentID: document.id, service: service)
        self.document = document
        self.service = service
        self.correspondent = correspondent
        documentType = type
        summary = DocumentSummary(document, correspondent: correspondent, documentType: type)
        rebuildFields()
    }

    /// Takes newer list data for the same document without restarting its preview.
    public func update(document updated: Document, correspondent: String?, type: String?) {
        guard updated.id == document.id else { return }
        self.correspondent = correspondent
        documentType = type
        if updated.updatedAt > document.updatedAt {
            document = updated
        }
        rebuildFields()
    }

    private func rebuildFields() {
        var fields = [
            MetadataField(label: "From", value: correspondent ?? "Not set", isRecord: false),
            MetadataField(label: "Type", value: documentType ?? "Not set", isRecord: false),
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
        summary = DocumentSummary(document, correspondent: correspondent, documentType: documentType)
    }

    public var reviewDescription: String? {
        guard let reviewed = document.reviewed else { return nil }
        guard reviewed else { return "Not reviewed" }
        if document.reviewOrigin == "legacy_classification" { return "Filed before review tracking" }
        if document.reviewOrigin == "system" { return "Reviewed by system" }
        if let reviewer = document.reviewedBy { return "Reviewed by \(reviewer)" }
        return "Reviewed"
    }

    public func saveReview(_ reviewed: Bool) async throws {
        guard document.canReviewDocument else {
            throw FolioError.server(status: 403, message: "Review is unavailable for this document.")
        }
        guard !savingReview else { return }
        savingReview = true
        reviewFailure = nil
        defer { savingReview = false }
        do {
            applyMetadata(try await service.setDocumentReview(reviewed, for: document.id))
        } catch {
            reviewFailure = error.localizedDescription
            throw error
        }
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
        rebuildFields()
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
