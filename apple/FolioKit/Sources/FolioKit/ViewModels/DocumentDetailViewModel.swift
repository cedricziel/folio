import Foundation
import Observation

/// Names for the correspondent and document type IDs a document refers to.
public struct DocumentLookups: Equatable, Sendable {
    public var correspondents: [Correspondent.ID: String]
    public var documentTypes: [DocumentType.ID: String]

    public init(
        correspondents: [Correspondent.ID: String] = [:], documentTypes: [DocumentType.ID: String] = [:]
    ) {
        self.correspondents = correspondents
        self.documentTypes = documentTypes
    }
}

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
    private var lookups: DocumentLookups
    private var loading = false
    private var metadataWrites = 0

    public init(document: Document, lookups: DocumentLookups = DocumentLookups(), service: any FolioService) {
        discussion = DocumentDiscussionViewModel(documentID: document.id, service: service)
        self.document = document
        self.service = service
        self.lookups = lookups
        summary = DocumentSummary(document, correspondent: nil, documentType: nil)
        rebuildFields()
    }

    /// Takes newer list data for the same document without restarting its preview.
    ///
    /// List rows from a search carry no tags, so a row without tags keeps the ones already shown
    /// until the full document is fetched again.
    public func update(document row: Document, lookups: DocumentLookups) {
        guard row.id == document.id else { return }
        self.lookups = lookups
        if row.updatedAt > document.updatedAt {
            var merged = row
            if merged.tags.isEmpty { merged.tags = document.tags }
            document = merged
        }
        rebuildFields()
    }

    private func rebuildFields() {
        let correspondent = document.correspondentID.flatMap { lookups.correspondents[$0] }
        let documentType = document.documentTypeID.flatMap { lookups.documentTypes[$0] }
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
            let updated = try await service.setDocumentReview(reviewed, for: document.id)
            metadataWrites += 1
            applyMetadata(updated)
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

    /// Fetches the full document; a superseded or cancelled fetch changes nothing.
    public func refreshMetadata() async {
        let writes = metadataWrites
        do {
            let updated = try await service.document(id: document.id)
            guard !Task.isCancelled, writes == metadataWrites else { return }
            applyMetadata(updated)
            metadataFailure = nil
        } catch {
            // The live service reports a cancelled request as unreachable.
            guard !Task.isCancelled, !(error is CancellationError) else { return }
            metadataFailure = error.localizedDescription
        }
    }

    private func applyMetadata(_ updated: Document) {
        guard updated.updatedAt >= document.updatedAt else { return }
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
