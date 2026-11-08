import AppIntents
import CoreSpotlight
import CryptoKit
import Foundation

/// One document's Spotlight entry: the vault link opens it, the domain scopes it to a server and document.
public struct SpotlightItem: Sendable, Equatable {
    public let identifier: String
    public let domain: String
    public let summary: DocumentSummary

    public init(identifier: String, domain: String, summary: DocumentSummary) {
        self.identifier = identifier
        self.domain = domain
        self.summary = summary
    }

    public init(_ summary: DocumentSummary, vaultID: UUID, server: URL) {
        var summary = summary
        summary.vaultID = vaultID
        self.init(
            identifier: DocumentLink(documentID: summary.id, vaultID: vaultID).spotlightIdentifier,
            domain: SpotlightIndex.domain(for: server, documentID: summary.id),
            summary: summary
        )
    }

    public var documentID: Document.ID { summary.id }

    /// Carries the document's entity, so Spotlight can offer Folio's document actions on the result.
    public var searchableItem: CSSearchableItem {
        let entity = DocumentEntity(summary)
        let attributes = entity.attributeSet
        attributes.associateAppEntity(entity)
        return CSSearchableItem(uniqueIdentifier: identifier, domainIdentifier: domain, attributeSet: attributes)
    }
}

/// The system calls Folio makes on the Spotlight index.
public protocol SpotlightIndexing: Sendable {
    func index(_ items: [SpotlightItem]) async throws
    /// Deletes the items in these domains and their subdomains.
    func deleteDomains(_ domains: [String]) async throws
}

struct SystemSpotlightIndex: SpotlightIndexing {
    func index(_ items: [SpotlightItem]) async throws {
        try await CSSearchableIndex.default().indexSearchableItems(items.map(\.searchableItem))
    }

    func deleteDomains(_ domains: [String]) async throws {
        try await CSSearchableIndex.default().deleteSearchableItems(withDomainIdentifiers: domains)
    }
}

/// Keeps Spotlight entries scoped to the signed-in account and server and clears them when access ends.
///
/// Credentials carry no account identity, so an interactive sign-in always starts from an empty index: that is
/// the only way a different account on the same server can begin a session. Restoring a saved session keeps
/// the entries of the server they were indexed for.
///
/// Every index call runs in order on one queue, so a clear always lands after any donation requested before it,
/// and a donation runs only if the session it was requested in is still the current one.
public final class SpotlightIndex: @unchecked Sendable {
    public static let shared = SpotlightIndex(index: SystemSpotlightIndex(), defaults: .standard)
    /// How many documents listing adds to a server's index; opened documents are always indexed.
    public static let defaultLibraryLimit = 1000

    static let rootDomain = "com.cedricziel.folio.documents"
    private static let indexedServerKey = "spotlightIndexedServer"
    private static let libraryKey = "spotlightIndexedDocuments"

    /// A signed-in session; donations made under an older scope are dropped.
    public struct Scope: Sendable, Equatable {
        let generation: Int
        let server: URL
    }

    private let index: any SpotlightIndexing
    private let defaults: UserDefaults
    private let libraryLimit: Int
    private let lock = NSLock()
    private var generation = 0
    private var server: URL?
    /// The server the entries belong to; outlives a transient suspension so confirmed deletes still apply.
    private var indexedServer: URL?
    private var tail: Task<Void, Never>?
    /// What this session already sent, so relisting unchanged documents costs nothing.
    private var sent: (generation: Int, items: [Document.ID: SpotlightItem]) = (0, [:])
    /// Every document indexed for a server, kept across launches so the limit bounds the whole index.
    private var library: (server: URL, ids: Set<Document.ID>)?

    public init(index: any SpotlightIndexing, defaults: UserDefaults, libraryLimit: Int = defaultLibraryLimit) {
        self.index = index
        self.defaults = defaults
        self.libraryLimit = libraryLimit
    }

    static func domain(for server: URL) -> String {
        let digest = SHA256.hash(data: Data(server.absoluteString.utf8))
        return rootDomain + "." + digest.prefix(16).map { String(format: "%02x", $0) }.joined()
    }

    static func domain(for server: URL, documentID: Document.ID) -> String {
        domain(for: server) + "." + documentID.uuidString.lowercased()
    }

    public var currentScope: Scope? {
        lock.withLock { server.map { Scope(generation: generation, server: $0) } }
    }

    /// Follows the app's session. Signed in scopes donations to that server. Signed out and expired clear every
    /// entry: an expired session was revoked, so its metadata must not stay searchable until the next sign-in.
    /// Restoring, interrupted and unavailable are transient, so they only pause donations.
    @discardableResult
    public func sessionChanged(to session: SessionRestoration) -> Task<Void, Never> {
        switch session {
        case let .signedIn(url): activate(url)
        case .signedOut, .expired: clear()
        case .restoring, .interrupted, .unavailable: suspend()
        }
    }

    public func donate(_ summary: DocumentSummary, vaultID: UUID) async {
        guard !Task.isCancelled, let scope = currentScope else { return }
        await donate(summary, vaultID: vaultID, scope: scope).value
    }

    @discardableResult
    public func donate(_ summary: DocumentSummary, vaultID: UUID, scope: Scope) -> Task<Void, Never> {
        let item = SpotlightItem(summary, vaultID: vaultID, server: scope.server)
        lock.withLock {
            guard isCurrent(scope) else { return }
            record(item)
            var ids = libraryIDs(for: scope.server)
            if ids.insert(summary.id).inserted { setLibraryIDs(ids, for: scope.server) }
        }
        return send([item], scope: scope)
    }

    /// Indexes documents the app listed under the current session.
    @discardableResult
    public func index(_ summaries: [DocumentSummary], vaultID: UUID) -> Task<Void, Never> {
        index(summaries, vaultID: vaultID, scope: currentScope)
    }

    /// Indexes documents the app listed, so Spotlight finds more than the ones someone opened. `scope` is the
    /// session the list was requested in; a list that outlived its session is dropped. Unchanged documents are
    /// skipped, and new ones stop once the server's index holds the library limit.
    @discardableResult
    public func index(_ summaries: [DocumentSummary], vaultID: UUID, scope: Scope?) -> Task<Void, Never> {
        let items: [SpotlightItem] = lock.withLock {
            guard let scope, isCurrent(scope) else { return [] }
            var ids = libraryIDs(for: scope.server)
            let items = summaries.compactMap { summary -> SpotlightItem? in
                let item = SpotlightItem(summary, vaultID: vaultID, server: scope.server)
                guard sentItems[summary.id] != item,
                    ids.contains(summary.id) || ids.count < libraryLimit
                else { return nil }
                ids.insert(summary.id)
                record(item)
                return item
            }
            if !items.isEmpty { setLibraryIDs(ids, for: scope.server) }
            return items
        }
        guard let scope, !items.isEmpty else { return Task {} }
        return send(items, scope: scope)
    }

    /// Starts an interactively signed-in session from an empty index, whichever account it belongs to.
    @discardableResult
    public func signedIn(to url: URL) -> Task<Void, Never> {
        lock.withLock {
            generation += 1
            server = url
            indexedServer = url
            resetLibrary()
        }
        return keepAlive(
            enqueue { index in
                self.defaults.removeObject(forKey: Self.indexedServerKey)
                guard (try? await index.deleteDomains([Self.rootDomain])) != nil else { return }
                self.defaults.set(url.absoluteString, forKey: Self.indexedServerKey)
            })
    }

    /// Removes a document the server deleted or no longer has.
    @discardableResult
    public func remove(documentID: Document.ID) -> Task<Void, Never> {
        remove(documentIDs: [documentID])
    }

    /// Removes documents the server deleted or no longer has. Before a saved session is restored, for example
    /// when an intent launches the app, `fallbackServer` names the server the caller looked them up on.
    @discardableResult
    public func remove(documentIDs: [Document.ID], fallbackServer: URL? = nil) -> Task<Void, Never> {
        let server: URL? = lock.withLock {
            guard let server = self.server ?? indexedServer ?? fallbackServer else { return nil }
            for id in documentIDs { sentItems[id] = nil }
            var ids = libraryIDs(for: server)
            if !ids.isDisjoint(with: documentIDs) {
                ids.subtract(documentIDs)
                setLibraryIDs(ids, for: server)
            }
            return server
        }
        guard let server, !documentIDs.isEmpty else { return Task {} }
        let domains = documentIDs.map { Self.domain(for: server, documentID: $0) }
        return enqueue { index in try? await index.deleteDomains(domains) }
    }

    /// Waits for every index call requested so far.
    public func idle() async {
        await enqueue { _ in }.value
    }

    private func activate(_ url: URL) -> Task<Void, Never> {
        let unchanged = lock.withLock {
            guard server != url else { return true }
            generation += 1
            server = url
            indexedServer = url
            return false
        }
        if unchanged { return Task {} }
        return enqueue { index in
            // Entries from another server, or from builds that did not scope them, must not outlive the switch.
            guard self.defaults.string(forKey: Self.indexedServerKey) != url.absoluteString else { return }
            // A failed clear leaves the old server recorded, so the next activation tries again.
            guard (try? await index.deleteDomains([Self.rootDomain])) != nil else { return }
            self.defaults.set(url.absoluteString, forKey: Self.indexedServerKey)
        }
    }

    private func clear() -> Task<Void, Never> {
        lock.withLock {
            generation += 1
            server = nil
            indexedServer = nil
            resetLibrary()
        }
        return keepAlive(
            enqueue { index in
                self.defaults.removeObject(forKey: Self.indexedServerKey)
                try? await index.deleteDomains([Self.rootDomain])
            })
    }

    /// On iOS, asks for time to finish a clear the app started just before moving to the background.
    private func keepAlive(_ task: Task<Void, Never>) -> Task<Void, Never> {
        #if os(iOS)
            let reason = "Remove Folio documents from Spotlight"
            ProcessInfo.processInfo.performExpiringActivity(withReason: reason) { expired in
                guard !expired else { return }
                let finished = DispatchSemaphore(value: 0)
                Task {
                    await task.value
                    finished.signal()
                }
                finished.wait()
            }
        #endif
        return task
    }

    @discardableResult
    private func suspend() -> Task<Void, Never> {
        lock.withLock {
            generation += 1
            server = nil
        }
        return Task {}
    }

    /// The session's sent items; a new generation starts empty. Call with the lock held.
    private var sentItems: [Document.ID: SpotlightItem] {
        get { sent.generation == generation ? sent.items : [:] }
        set { sent = (generation, newValue) }
    }

    private func isCurrent(_ scope: Scope) -> Bool {
        scope.generation == generation && scope.server == server
    }

    private func record(_ item: SpotlightItem) {
        sentItems[item.documentID] = item
    }

    /// Call with the lock held.
    private func libraryIDs(for server: URL) -> Set<Document.ID> {
        if let library, library.server == server { return library.ids }
        let stored = defaults.dictionary(forKey: Self.libraryKey)
        let ids: Set<Document.ID> =
            stored?["server"] as? String == server.absoluteString
            ? Set((stored?["ids"] as? [String] ?? []).compactMap(UUID.init(uuidString:))) : []
        library = (server, ids)
        return ids
    }

    /// Call with the lock held.
    private func setLibraryIDs(_ ids: Set<Document.ID>, for server: URL) {
        library = (server, ids)
        defaults.set(
            ["server": server.absoluteString, "ids": ids.map(\.uuidString)] as [String: Any],
            forKey: Self.libraryKey)
    }

    /// Call with the lock held.
    private func resetLibrary() {
        library = nil
        defaults.removeObject(forKey: Self.libraryKey)
    }

    private func send(_ items: [SpotlightItem], scope: Scope) -> Task<Void, Never> {
        enqueue { index in
            guard self.currentScope == scope else { return }
            do {
                try await index.index(items)
            } catch {
                // Forget what failed, so the next listing tries again and it holds no place under the limit.
                self.lock.withLock {
                    guard self.isCurrent(scope) else { return }
                    let failed = items.filter { self.sentItems[$0.documentID] == $0 }.map(\.documentID)
                    for id in failed { self.sentItems[id] = nil }
                    var ids = self.libraryIDs(for: scope.server)
                    ids.subtract(failed)
                    self.setLibraryIDs(ids, for: scope.server)
                }
            }
        }
    }

    private func enqueue(_ operation: @escaping @Sendable (any SpotlightIndexing) async -> Void) -> Task<Void, Never> {
        let index = index
        return lock.withLock {
            let previous = tail
            let task = Task {
                await previous?.value
                await operation(index)
            }
            tail = task
            return task
        }
    }
}

/// Re-indexes a document in Spotlight when someone opens it.
public enum SpotlightDonor {
    public static func donate(_ summary: DocumentSummary, vaultID: UUID) async {
        await SpotlightIndex.shared.donate(summary, vaultID: vaultID)
    }
}
