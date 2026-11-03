import Foundation
import Observation

@MainActor @Observable
public final class DocumentDiscussionViewModel {
    public private(set) var kind: DocumentEntryKind = .note
    public private(set) var entries: [DocumentEntry] = []
    public private(set) var nextCursor: String?
    public private(set) var canCreate = false
    public private(set) var isLoading = false
    public private(set) var isSaving = false
    public private(set) var isEditing = false
    public private(set) var creationKey: UUID?
    public private(set) var editingID: UUID?
    public private(set) var error: String?
    public var draft = ""
    private let documentID: UUID
    private let service: any DocumentEntryService
    private var generation = 0
    private var submittedBody: String?

    public init(documentID: UUID, service: any DocumentEntryService) {
        self.documentID = documentID
        self.service = service
    }

    public var canSave: Bool {
        !isSaving && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && draft.utf8.count <= 16_384
    }

    public func select(_ kind: DocumentEntryKind) async {
        guard !isEditing, self.kind != kind else { return }
        self.kind = kind
        generation += 1
        entries = []
        nextCursor = nil
        canCreate = false
        await load()
    }

    public func load() async { await fetch(cursor: nil) }
    public func loadMore() async {
        guard let nextCursor, !isLoading else { return }
        await fetch(cursor: nextCursor)
    }

    private func fetch(cursor: String?) async {
        generation += 1
        let current = generation
        isLoading = true
        error = nil
        do {
            let page = try await service.entryPage(documentID: documentID, kind: kind, cursor: cursor)
            guard current == generation else { return }
            if cursor == nil { entries = [] }
            merge(page.entries)
            nextCursor = page.nextCursor
            canCreate = page.canCreate
        } catch {
            guard current == generation else { return }
            if Self.status(of: error) == 404 { revokeAccess() }
            revokeAccessIfDenied(error)
            self.error = error.localizedDescription
        }
        if current == generation { isLoading = false }
    }

    public func beginAdding() {
        guard canCreate, !isSaving else { return }
        draft = ""
        editingID = nil
        creationKey = UUID()
        submittedBody = nil
        isEditing = true
        error = nil
    }

    public func beginEditing(_ entry: DocumentEntry) {
        guard entry.canEdit, !isSaving else { return }
        draft = entry.body
        editingID = entry.id
        creationKey = nil
        submittedBody = nil
        isEditing = true
        error = nil
    }

    public func cancel() {
        guard !isSaving else { return }
        if draftIsLocked { Task { await load() } }
        closeEditor()
    }

    private func closeEditor() {
        draft = ""
        creationKey = nil
        editingID = nil
        submittedBody = nil
        isEditing = false
        error = nil
    }

    public func save() async {
        guard isEditing, canSave else { return }
        isSaving = true
        error = nil
        do {
            let saved: DocumentEntry
            if let editingID {
                saved = try await service.updateEntry(documentID: documentID, entryID: editingID, body: draft)
            } else if let creationKey {
                // A lost response must retry the exact payload associated with its creation key.
                let body = submittedBody ?? draft
                submittedBody = body
                saved = try await service.createEntry(documentID: documentID, kind: kind, body: body, key: creationKey)
            } else {
                isSaving = false
                return
            }
            generation += 1
            isLoading = false
            merge([saved])
            isSaving = false
            closeEditor()
        } catch {
            isSaving = false
            let status = Self.status(of: error)
            if let editingID, status == 404 {
                entries.removeAll { $0.id == editingID }
                closeEditor()
            } else if creationKey != nil, let status, (400..<500).contains(status), ![408, 409, 425, 429].contains(status) {
                creationKey = UUID()
                submittedBody = nil
            }
            revokeAccessIfDenied(error)
            self.error = error.localizedDescription
        }
    }

    private func merge(_ incoming: [DocumentEntry]) {
        var byID = Dictionary(uniqueKeysWithValues: entries.map { ($0.id, $0) })
        for entry in incoming { byID[entry.id] = entry }
        entries = byID.values.sorted {
            if $0.createdAt == $1.createdAt { return $0.id.uuidString < $1.id.uuidString }
            return $0.createdAt < $1.createdAt
        }
    }

    public var draftIsLocked: Bool { submittedBody != nil }

    public func delete(_ entry: DocumentEntry) async {
        guard entry.canDelete, !isSaving else { return }
        isSaving = true
        error = nil
        do {
            try await service.deleteEntry(documentID: documentID, entryID: entry.id)
            generation += 1
            isLoading = false
            entries.removeAll { $0.id == entry.id }
        } catch {
            if Self.status(of: error) == 404 {
                entries.removeAll { $0.id == entry.id }
            } else {
                revokeAccessIfDenied(error)
                self.error = error.localizedDescription
            }
        }
        isSaving = false
    }

    private static func status(of error: any Error) -> Int? {
        if case .server(let status, _) = error as? FolioError { return status }
        return nil
    }

    private func revokeAccessIfDenied(_ error: any Error) {
        if case .signInRequired = error as? FolioError { revokeAccess() }
        if Self.status(of: error) == 403 { revokeAccess() }
    }

    private func revokeAccess() {
        entries = []
        nextCursor = nil
        canCreate = false
    }
}
