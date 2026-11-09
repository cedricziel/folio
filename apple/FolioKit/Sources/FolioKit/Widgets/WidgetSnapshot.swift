import Foundation

/// What the widgets show. The app writes it to the App Group; widgets only read it, so they never talk to the server.
public struct WidgetSnapshot: Codable, Equatable, Sendable {
    public struct Latest: Codable, Equatable, Sendable {
        public var documentID: Document.ID
        public var vaultID: UUID
        /// Nil while App lock is on, so titles never leave the app.
        public var title: String?
        public var createdAt: Date

        public init(documentID: Document.ID, vaultID: UUID, title: String?, createdAt: Date) {
            self.documentID = documentID
            self.vaultID = vaultID
            self.title = title
            self.createdAt = createdAt
        }
    }

    public var isSignedIn: Bool
    public var inboxCount: Int
    /// More documents may need review than the app counted.
    public var inboxCountIsPartial: Bool
    public var latest: Latest?
    public var hidesTitles: Bool
    public var updatedAt: Date

    public init(
        isSignedIn: Bool, inboxCount: Int, inboxCountIsPartial: Bool, latest: Latest?, hidesTitles: Bool,
        updatedAt: Date
    ) {
        self.isSignedIn = isSignedIn
        self.inboxCount = inboxCount
        self.inboxCountIsPartial = inboxCountIsPartial
        self.latest = latest
        self.hidesTitles = hidesTitles
        self.updatedAt = updatedAt
    }

    public static func signedOut(at date: Date) -> Self {
        Self(
            isSignedIn: false, inboxCount: 0, inboxCountIsPartial: false, latest: nil, hidesTitles: true,
            updatedAt: date)
    }

    public func hidingTitles() -> Self {
        var hidden = self
        hidden.latest?.title = nil
        hidden.hidesTitles = true
        return hidden
    }

    public var inboxCountText: String {
        inboxCountIsPartial ? "\(inboxCount)+" : "\(inboxCount)"
    }
}

public struct WidgetSnapshotStore: Sendable {
    static let key = "widgetSnapshot"
    private nonisolated(unsafe) let defaults: UserDefaults

    public init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    public static func shared() -> Self {
        Self(defaults: UserDefaults(suiteName: SharedConfiguration.appGroup) ?? .standard)
    }

    public func load() -> WidgetSnapshot? {
        defaults.data(forKey: Self.key).flatMap { try? JSONDecoder().decode(WidgetSnapshot.self, from: $0) }
    }

    public func save(_ snapshot: WidgetSnapshot) {
        defaults.set(try? JSONEncoder().encode(snapshot), forKey: Self.key)
    }

    public func hideTitles() {
        guard let snapshot = load() else { return }
        save(snapshot.hidingTitles())
    }
}

public enum WidgetSnapshotBuilder {
    static let pageLimit: Int32 = 100
    /// Bounds the work on every app activation; past it the count is what was found, followed by "+".
    static let maxPages = 3

    /// Counts what the Inbox shows (documents not yet filed) in the server's default vault.
    public static func make(from service: any FolioService, revealsTitles: Bool, now: Date = Date()) async throws
        -> WidgetSnapshot
    {
        var cursor: String?
        var count = 0
        var newest: (document: Document, vaultID: UUID)?
        for _ in 0..<maxPages {
            let page = try await service.documentPage(
                query: DocumentBrowseQuery(sort: .newest, limit: pageLimit), cursor: cursor)
            if newest == nil, let first = page.documents.first { newest = (first, page.vaultID) }
            count += page.documents.filter { $0.status != .filed }.count
            cursor = page.nextCursor
            if cursor == nil { break }
        }
        let latest = newest.map {
            WidgetSnapshot.Latest(
                documentID: $0.document.id, vaultID: $0.vaultID, title: $0.document.title,
                createdAt: $0.document.createdAt)
        }
        let snapshot = WidgetSnapshot(
            isSignedIn: true, inboxCount: count, inboxCountIsPartial: cursor != nil, latest: latest,
            hidesTitles: false, updatedAt: now)
        return revealsTitles ? snapshot : snapshot.hidingTitles()
    }
}
