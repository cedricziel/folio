import Foundation
import Observation

public struct DocumentRow: Identifiable, Equatable, Sendable {
    public let document: Document
    public let subtitle: String?
    public let dateText: String
    public let statusText: String?

    public var id: Document.ID {
        document.id
    }

    public var title: String {
        document.title
    }

    public var status: Document.Status {
        document.status
    }
}

@MainActor @Observable
public final class DocumentListViewModel {
    public enum Phase: Equatable {
        case idle
        case loading
        case loaded
        case failed(String)
    }

    public private(set) var phase = Phase.idle
    public private(set) var rows: [DocumentRow] = []
    public private(set) var correspondents: [Correspondent.ID: String] = [:]
    public private(set) var documentTypes: [DocumentType.ID: String] = [:]
    public var searchText = ""

    private let service: any FolioService
    private let searchDebounce: Duration
    private var documents: [Document] = []

    public init(service: any FolioService, searchDebounce: Duration = .milliseconds(300)) {
        self.service = service
        self.searchDebounce = searchDebounce
    }

    public func correspondentName(for document: Document) -> String? {
        document.correspondentID.flatMap { correspondents[$0] }
    }

    public func typeName(for document: Document) -> String? {
        document.documentTypeID.flatMap { documentTypes[$0] }
    }

    /// Initial load and pull to refresh.
    public func load() async {
        if rows.isEmpty {
            phase = .loading
        }
        async let lookups: Void = loadLookups()
        await fetch(query: searchText)
        await lookups
        rebuildRows()
    }

    /// Call from `.task(id: searchText)`: SwiftUI cancels the previous task when the text changes,
    /// so only the last keystroke in a burst reaches the server.
    public func search(_ text: String) async {
        let query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !query.isEmpty {
            try? await Task.sleep(for: searchDebounce)
        }
        guard !Task.isCancelled else { return }
        await fetch(query: query)
        rebuildRows()
    }

    public func delete(_ document: Document) async {
        do {
            try await service.deleteDocument(id: document.id)
            documents.removeAll { $0.id == document.id }
            rebuildRows()
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    private func fetch(query: String) async {
        do {
            let result = try await service.documents(matching: query)
            guard !Task.isCancelled else { return }
            documents = result
            phase = .loaded
        } catch is CancellationError {
            return
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    private func loadLookups() async {
        async let people = try? service.correspondents()
        async let types = try? service.documentTypes()
        let (loadedPeople, loadedTypes) = await (people, types)
        if let loadedPeople {
            correspondents = Dictionary(uniqueKeysWithValues: loadedPeople.map { ($0.id, $0.name) })
        }
        if let loadedTypes {
            documentTypes = Dictionary(uniqueKeysWithValues: loadedTypes.map { ($0.id, $0.name) })
        }
    }

    private func rebuildRows() {
        rows = documents.map { document in
            DocumentRow(
                document: document,
                subtitle: [correspondentName(for: document), typeName(for: document)]
                    .compactMap { $0 }.joined(separator: " · ").nilIfEmpty,
                dateText: FolioFormat.isoDay(document.createdAt),
                statusText: FolioFormat.statusText(document.status)
            )
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
