import Foundation

/// Putting a tag on documents, or taking it off.
public struct TagChange: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case add, remove
    }

    public var kind: Kind
    public var tagID: Tag.ID
    /// The vault holding the tag; nil fits any vault.
    public var tagVaultID: UUID?
    public var documents: [DocumentEntityID]

    public init(_ kind: Kind, tagID: Tag.ID, tagVaultID: UUID? = nil, documents: [DocumentEntityID]) {
        self.kind = kind
        self.tagID = tagID
        self.tagVaultID = tagVaultID
        self.documents = documents
    }

    /// The change that puts the same documents back.
    public var reverted: TagChange {
        TagChange(kind == .add ? .remove : .add, tagID: tagID, tagVaultID: tagVaultID, documents: documents)
    }
}

/// What applying a ``TagChange`` did to each document.
public struct TagChangeOutcome: Equatable, Sendable {
    /// The change limited to the documents it altered, which is what undo reverts.
    public var applied: TagChange
    /// Documents that already had the tag, or already lacked it.
    public var unchanged: [DocumentEntityID] = []
    /// Documents in another vault than the tag.
    public var otherVault: [DocumentEntityID] = []
    /// Documents that are gone, or that the server refused or didn't answer for.
    public var failed: [DocumentEntityID] = []

    /// One sentence for the dialog, naming the tag.
    public func summary(tagName: String) -> String {
        let count = applied.documents.count
        let documents = "\(count) document\(count == 1 ? "" : "s")"
        var sentences: [String] = []
        if count > 0 || (unchanged.isEmpty && otherVault.isEmpty && failed.isEmpty) {
            sentences.append(
                applied.kind == .add ? "Tagged \(documents) with \(tagName)." : "Removed \(tagName) from \(documents).")
        }
        if !unchanged.isEmpty {
            let verb = applied.kind == .add ? "already had it" : "didn't have it"
            sentences.append("\(unchanged.count) \(verb).")
        }
        if !otherVault.isEmpty {
            sentences.append("\(otherVault.count) \(otherVault.count == 1 ? "is" : "are") in another vault.")
        }
        if !failed.isEmpty {
            sentences.append("\(failed.count) couldn't be changed.")
        }
        return sentences.joined(separator: " ")
    }
}

extension DocumentCatalog {
    /// Adds or removes the tag on each document that needs it, a few at a time. One request per vault lists the IDs
    /// of the documents already carrying the tag, so the documents themselves are never fetched. Throws only when every
    /// document failed.
    public func apply(_ change: TagChange, concurrency: Int = 4) async throws -> TagChangeOutcome {
        var applied = change
        applied.documents = []
        var outcome = TagChangeOutcome(applied: applied)
        var firstError: (any Error)?

        var vaultOrder: [UUID?] = []
        var byVault: [UUID?: [DocumentEntityID]] = [:]
        var seen = Set<DocumentEntityID>()
        for id in change.documents where seen.insert(id).inserted {
            if let tagVault = change.tagVaultID, let documentVault = id.vaultID, tagVault != documentVault {
                outcome.otherVault.append(id)
                continue
            }
            // An id without a vault is looked up where the tag lives, the only vault the change can succeed in.
            let vault = id.vaultID ?? change.tagVaultID
            if byVault[vault] == nil { vaultOrder.append(vault) }
            byVault[vault, default: []].append(id)
        }

        var targets: [DocumentEntityID] = []
        for vault in vaultOrder {
            let ids = byVault[vault] ?? []
            let tagged: Set<Document.ID>
            do {
                let filter = DocumentFilter(tagIDs: [change.tagID])
                tagged = Set(try await scoped(to: vault).service.documentIDs(filter: filter))
            } catch {
                firstError = firstError ?? error
                outcome.failed += ids
                continue
            }
            for id in ids {
                if tagged.contains(id.documentID) == (change.kind == .add) {
                    outcome.unchanged.append(id)
                } else {
                    targets.append(id)
                }
            }
        }

        var results = [Result<Void, any Error>?](repeating: nil, count: targets.count)
        await withTaskGroup(of: (Int, Result<Void, any Error>).self) { group in
            var next = 0
            func start() {
                guard next < targets.count else { return }
                let position = next
                let catalog = scoped(to: targets[position].vaultID ?? change.tagVaultID)
                let documentID = targets[position].documentID
                group.addTask {
                    do {
                        switch change.kind {
                        case .add: try await catalog.addTag(change.tagID, to: documentID)
                        case .remove: try await catalog.removeTag(change.tagID, from: documentID)
                        }
                        return (position, .success(()))
                    } catch {
                        return (position, .failure(error))
                    }
                }
                next += 1
            }
            for _ in 0..<max(1, concurrency) { start() }
            for await (position, result) in group {
                results[position] = result
                start()
            }
        }
        for (id, result) in zip(targets, results) {
            switch result {
            case .success?:
                outcome.applied.documents.append(id)
            case let .failure(error)?:
                firstError = firstError ?? error
                outcome.failed.append(id)
            case nil:
                outcome.failed.append(id)
            }
        }
        if let firstError, outcome.failed.count == seen.count { throw firstError }
        return outcome
    }
}
