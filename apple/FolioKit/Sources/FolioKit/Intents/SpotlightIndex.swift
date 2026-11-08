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
        self.init(
            identifier: DocumentLink(documentID: summary.id, vaultID: vaultID).spotlightIdentifier,
            domain: SpotlightIndex.domain(for: server, documentID: summary.id),
            summary: summary
        )
    }

    public var documentID: Document.ID { summary.id }

    public var searchableItem: CSSearchableItem {
        CSSearchableItem(
            uniqueIdentifier: identifier, domainIdentifier: domain, attributeSet: DocumentEntity(summary).attributeSet)
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

    static let rootDomain = "com.cedricziel.folio.documents"
    private static let indexedServerKey = "spotlightIndexedServer"

    /// A signed-in session; donations made under an older scope are dropped.
    public struct Scope: Sendable, Equatable {
        let generation: Int
        let server: URL
    }

    private let index: any SpotlightIndexing
    private let defaults: UserDefaults
    private let lock = NSLock()
    private var generation = 0
    private var server: URL?
    /// The server the entries belong to; outlives a transient suspension so confirmed deletes still apply.
    private var indexedServer: URL?
    private var tail: Task<Void, Never>?

    public init(index: any SpotlightIndexing, defaults: UserDefaults) {
        self.index = index
        self.defaults = defaults
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
        return enqueue { index in
            guard self.currentScope == scope else { return }
            try? await index.index([item])
        }
    }

    /// Starts an interactively signed-in session from an empty index, whichever account it belongs to.
    @discardableResult
    public func signedIn(to url: URL) -> Task<Void, Never> {
        lock.withLock {
            generation += 1
            server = url
            indexedServer = url
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
        guard let server = lock.withLock({ server ?? indexedServer }) else { return Task {} }
        let domain = Self.domain(for: server, documentID: documentID)
        return enqueue { index in try? await index.deleteDomains([domain]) }
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
