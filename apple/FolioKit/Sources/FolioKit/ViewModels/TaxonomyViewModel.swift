import Foundation
import Observation

@MainActor @Observable
public final class TaxonomyViewModel {
    public private(set) var entries: [TaxonomyEntry] = []
    public private(set) var trash: [TaxonomyEntry] = []
    public private(set) var error: String?
    public private(set) var isBusy = false
    public let kind: TaxonomyKind
    private let service: any FolioService

    public init(service: any FolioService, kind: TaxonomyKind) {
        self.service = service
        self.kind = kind
    }

    public func load() async {
        guard !isBusy else { return }
        isBusy = true
        defer { isBusy = false }
        do {
            async let entries = service.taxonomy(kind, deleted: false)
            async let trash = service.taxonomy(kind, deleted: true)
            let (active, deleted) = try await (entries, trash)
            guard !Task.isCancelled else { return }
            self.entries = active
            self.trash = deleted
            error = nil
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    public func save(name: String, id: UUID? = nil) async {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        await mutate { try await self.service.saveTaxonomy(self.kind, id: id, name: name) }
    }

    public func delete(_ entry: TaxonomyEntry) async {
        await mutate { try await self.service.deleteTaxonomy(self.kind, id: entry.id) }
    }

    public func restore(_ entry: TaxonomyEntry) async {
        await mutate(conflict: "Restore failed. An active entry already uses this name.") {
            try await self.service.restoreTaxonomy(self.kind, id: entry.id)
        }
    }

    private func mutate(
        conflict: String = "An active entry already uses this name.", _ operation: () async throws -> Void
    ) async {
        guard !isBusy else { return }
        isBusy = true
        error = nil
        do {
            try await operation()
            isBusy = false
            await load()
        } catch FolioError.server(status: 409, message: _) {
            error = conflict
            isBusy = false
        } catch {
            self.error = error.localizedDescription
            isBusy = false
        }
    }
}
