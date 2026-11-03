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

    public var tags: [Tag] {
        document.tags
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

    /// Which documents the list shows: everything, or only those still waiting for a look.
    public enum Scope: Equatable, Sendable {
        case inbox
        case all
    }

    public private(set) var phase = Phase.idle
    public private(set) var rows: [DocumentRow] = []
    /// Changes only for visible insertions after uploads or restores, and confirmed deletions.
    public private(set) var rowChangeRevision = 0
    public private(set) var correspondents: [Correspondent.ID: String] = [:]
    public private(set) var documentTypes: [DocumentType.ID: String] = [:]
    public var searchText = ""
    public var scope = Scope.all {
        didSet {
            guard scope != oldValue else { return }
            latestFetch += 1
            nextCursor = nil
            activeQuery = nil
            isLoadingMore = false
            rebuildRows()
        }
    }
    /// A failed action such as delete; shown transiently, the list stays as it was.
    public private(set) var actionError: String?

    public private(set) var supportsReviewFilters = false
    public var filter = DocumentFilter()
    public var sort = DocumentBrowseQuery.Sort.newest
    public private(set) var isLoadingMore = false
    public private(set) var pageError: String?

    public var query: DocumentBrowseQuery {
        .init(text: searchText.trimmingCharacters(in: .whitespacesAndNewlines), filter: filter, sort: sort)
    }

    public var canLoadMore: Bool {
        phase == .loaded && nextCursor != nil && activeQuery == query
    }

    private var nextCursor: String?
    private var activeQuery: DocumentBrowseQuery?
    private var loadedVaultID: UUID?
    private var resolvedDocument: Document?
    private var latestLookup = 0
    private var lookupID: Document.ID?
    private var lookupError: (id: Document.ID, message: String)?
    private let service: any FolioService
    private let searchDebounce: Duration
    private var documents: [Document] = []
    private var lookupsLoaded = false
    private var latestFetch = 0

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

    /// Reloads the current query; upload completion marks newly inserted rows for presentation.
    public func load(afterUpload: Bool = false) async {
        await reload(afterUpload: afterUpload)
    }

    private func reload(afterUpload: Bool = false, restoredDocumentID: Document.ID? = nil) async {
        let request = query
        let ticket = begin(request)
        async let lookups: Void = loadLookups()
        await fetch(request, ticket: ticket, afterUpload: afterUpload, restoredDocumentID: restoredDocumentID)
        await lookups
        if isCurrent(ticket) { rebuildRows() }
    }

    public func search(_ text: String) async {
        searchText = text
        let request = query
        let ticket = begin(request)
        if !request.text.isEmpty { try? await Task.sleep(for: searchDebounce) }
        guard isCurrent(ticket) else { return }
        if !lookupsLoaded { await loadLookups() }
        guard isCurrent(ticket) else { return }
        await fetch(request, ticket: ticket)
    }

    public func loadNextPage() async {
        guard canLoadMore, !isLoadingMore, let cursor = nextCursor, let request = activeQuery else { return }
        let ticket = latestFetch
        isLoadingMore = true
        pageError = nil
        defer { if ticket == latestFetch { isLoadingMore = false } }
        do {
            let page = try await readPage(request, cursor: cursor, ticket: ticket, knownIDs: Set(documents.map(\.id)))
            guard isCurrent(ticket) else { return }
            documents.append(contentsOf: page.documents)
            nextCursor = page.nextCursor
            rebuildRows()
        } catch {
            guard isCurrent(ticket) else { return }
            pageError = error.localizedDescription
        }
    }

    public func document(for id: Document.ID) -> Document? {
        documents.first { $0.id == id } ?? (resolvedDocument?.id == id ? resolvedDocument : nil)
    }

    public func documentError(for id: Document.ID) -> String? {
        lookupError?.id == id ? lookupError?.message : nil
    }

    public func resolveDocument(_ id: Document.ID) async {
        guard document(for: id) == nil else { return }
        latestLookup += 1
        let ticket = latestLookup
        lookupID = id
        lookupError = nil
        do {
            async let fetched = service.document(id: id)
            if !lookupsLoaded { await loadLookups() }
            let document = try await fetched
            guard document.id == id else { throw FolioError.unreadableResponse }
            guard !Task.isCancelled, ticket == latestLookup else { return }
            resolvedDocument = document
        } catch {
            guard !Task.isCancelled, ticket == latestLookup else { return }
            lookupError = (id, error.localizedDescription)
        }
    }

    @discardableResult
    public func delete(_ document: Document) async -> Bool {
        actionError = nil
        do {
            try await service.deleteDocument(id: document.id)
            let wasVisible = rows.contains { $0.id == document.id }
            documents.removeAll { $0.id == document.id }
            if resolvedDocument?.id == document.id {
                resolvedDocument = nil
            }
            if lookupID == document.id {
                latestLookup += 1
            }
            lookupError = (document.id, "This document is no longer available.")
            rebuildRows()
            if wasVisible { rowChangeRevision += 1 }
            return true
        } catch {
            actionError = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func restore(_ document: Document) async -> Bool {
        actionError = nil
        do {
            resolvedDocument = try await service.restoreDocument(id: document.id)
            if lookupError?.id == document.id {
                lookupError = nil
            }
            await reload(restoredDocumentID: document.id)
            return true
        } catch {
            actionError = error.localizedDescription
            return false
        }
    }

    public func dismissActionError() {
        actionError = nil
    }

    private func begin(_ request: DocumentBrowseQuery) -> Int {
        latestFetch += 1
        activeQuery = request
        nextCursor = nil
        loadedVaultID = nil
        pageError = nil
        isLoadingMore = false
        phase = .loading
        return latestFetch
    }

    private func isCurrent(_ ticket: Int) -> Bool {
        !Task.isCancelled && ticket == latestFetch
    }

    private func fetch(
        _ request: DocumentBrowseQuery, ticket: Int, afterUpload: Bool = false,
        restoredDocumentID: Document.ID? = nil
    ) async {
        do {
            let page = try await readPage(request, cursor: nil, ticket: ticket, knownIDs: [])
            guard isCurrent(ticket) else { return }
            let previousIDs = Set(rows.map(\.id))
            supportsReviewFilters = page.supportsReviewFilters
            documents = page.documents
            nextCursor = page.nextCursor
            loadedVaultID = page.vaultID
            phase = .loaded
            rebuildRows()
            if rows.contains(where: { !previousIDs.contains($0.id) && (afterUpload || $0.id == restoredDocumentID) }) {
                rowChangeRevision += 1
            }
        } catch {
            guard isCurrent(ticket) else { return }
            phase = .failed(error.localizedDescription)
        }
    }

    private func readPage(
        _ request: DocumentBrowseQuery, cursor initialCursor: String?, ticket: Int, knownIDs: Set<Document.ID>
    ) async throws -> DocumentPage {
        var cursor = initialCursor
        var seen = Set<String>()
        var vault = loadedVaultID
        while true {
            guard isCurrent(ticket) else { throw CancellationError() }
            if let cursor, !seen.insert(cursor).inserted { throw FolioError.unreadableResponse }
            let page = try await service.documentPage(query: request, cursor: cursor)
            guard isCurrent(ticket) else { throw CancellationError() }
            if let vault, vault != page.vaultID { throw FolioError.unreadableResponse }
            vault = page.vaultID
            if let next = page.nextCursor, seen.contains(next) { throw FolioError.unreadableResponse }
            var ids = knownIDs
            let fresh = page.documents.filter {
                (scope == .all || $0.status != .filed) && ids.insert($0.id).inserted
            }
            if !fresh.isEmpty || page.nextCursor == nil {
                return DocumentPage(
                    vaultID: page.vaultID, documents: fresh, nextCursor: page.nextCursor,
                    supportsReviewFilters: page.supportsReviewFilters)
            }
            cursor = page.nextCursor
        }
    }

    private func loadLookups() async {
        async let people = try? service.correspondents()
        async let types = try? service.documentTypes()
        let (loadedPeople, loadedTypes) = await (people, types)
        lookupsLoaded = loadedPeople != nil && loadedTypes != nil
        if let loadedPeople {
            correspondents = Dictionary(uniqueKeysWithValues: loadedPeople.map { ($0.id, $0.name) })
        }
        if let loadedTypes {
            documentTypes = Dictionary(uniqueKeysWithValues: loadedTypes.map { ($0.id, $0.name) })
        }
    }

    private func rebuildRows() {
        let shown = scope == .inbox ? documents.filter { $0.status != .filed } : documents
        rows = shown.map { document in
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
