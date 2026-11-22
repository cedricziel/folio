import Foundation
import Observation

@MainActor @Observable
public final class CustomMetadataViewModel {
    public private(set) var fields: [CustomFieldDefinition] = []
    public private(set) var snapshot = CustomMetadataSnapshot(revision: nil, values: [])
    public private(set) var error: String?
    public private(set) var isLoading = false
    public private(set) var hasMore = false
    public private(set) var isSaving = false
    public private(set) var needsReload = false
    private let documentID: UUID
    private let service: any CustomMetadataService
    private var loaded = false

    public init(documentID: UUID, service: any CustomMetadataService) {
        self.documentID = documentID
        self.service = service
    }

    public func load(more: Bool = false) async {
        guard !isLoading, !isSaving, more ? hasMore : !loaded else { return }
        isLoading = true
        defer { isLoading = false }
        error = nil
        do {
            let after = more ? fields.last?.id : nil
            async let page = service.customFields(after: after)
            if more {
                let next = try await page
                try Task.checkCancellation()
                fields += next
                hasMore = next.count == 50
            } else {
                async let current = service.customMetadata(documentID: documentID)
                let (next, metadata) = try await (page, current)
                try Task.checkCancellation()
                fields = next
                hasMore = next.count == 50
                snapshot = metadata
                loaded = true
            }
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    public func save(fieldID: UUID, value: CustomFieldValue?) async throws {
        guard loaded, !isSaving, !needsReload else {
            throw FolioError.server(status: 409, message: "Reload the current custom fields first.")
        }
        isSaving = true
        defer { isSaving = false }
        do {
            let updated = try await service.setCustomValue(
                documentID: documentID, revision: snapshot.revision, fieldID: fieldID, value: value
            )
            try Task.checkCancellation()
            snapshot = updated
        } catch let FolioError.server(status, message) where status == 409 {
            needsReload = true
            throw FolioError.server(status: status, message: message)
        }
    }

    public func reload() async throws {
        guard !isSaving else { return }
        let updated = try await service.customMetadata(documentID: documentID)
        try Task.checkCancellation()
        snapshot = updated
        needsReload = false
    }
}
