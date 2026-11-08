import AppIntents
import Foundation
import Observation
import os

/// The identifiers an open-document activity carries. Never credentials, contents, titles or file paths.
public struct DocumentActivityPayload: Equatable, Sendable {
    public static let activityType = "com.cedricziel.folio.document"
    public static let version = 1
    public static let requiredKeys: Set<String> = ["version", "server", "vault", "document"]

    public let server: URL
    public let vaultID: UUID
    public let documentID: Document.ID

    public init(server: URL, vaultID: UUID, documentID: Document.ID) {
        self.server = server
        self.vaultID = vaultID
        self.documentID = documentID
    }

    public var userInfo: [AnyHashable: Any] {
        [
            "version": Self.version,
            "server": server.absoluteString,
            "vault": vaultID.uuidString.lowercased(),
            "document": documentID.uuidString.lowercased(),
        ]
    }

    public init?(userInfo: [AnyHashable: Any]?) {
        guard let userInfo, userInfo["version"] as? Int == Self.version,
            let address = userInfo["server"] as? String, let server = URL(string: address),
            server.scheme == "https" || server.scheme == "http", server.host() != nil,
            let vault = (userInfo["vault"] as? String).flatMap(UUID.init(uuidString:)),
            let document = (userInfo["document"] as? String).flatMap(UUID.init(uuidString:))
        else { return nil }
        self.init(server: server, vaultID: vault, documentID: document)
    }
}

/// What the app knows about where open documents live, so intents and transfers that only receive a
/// document id reach the right server and vault. In memory only, and emptied when the session ends.
public final class DocumentLocator: Sendable {
    public static let shared = DocumentLocator()

    private struct State {
        var server: URL?
        var vaults: [Document.ID: UUID] = [:]
    }

    private let state = OSAllocatedUnfairLock(initialState: State())

    public init() {}

    public var server: URL? {
        state.withLock { $0.server }
    }

    /// Forgets every document and, when `server` is nil, stops producing links.
    public func reset(server: URL?) {
        state.withLock { $0 = State(server: server) }
    }

    public func record(documentID: Document.ID, vaultID: UUID) {
        state.withLock { state in
            guard state.server != nil else { return }
            state.vaults[documentID] = vaultID
        }
    }

    public func forget(documentID: Document.ID) {
        _ = state.withLock { $0.vaults.removeValue(forKey: documentID) }
    }

    public func vault(for documentID: Document.ID) -> UUID? {
        state.withLock { $0.vaults[documentID] }
    }

    /// The document's page in the web app, which asks for its own sign-in.
    public func webURL(for documentID: Document.ID) -> URL? {
        state.withLock { state in
            guard let server = state.server else { return nil }
            let document = "documents/\(documentID.uuidString.lowercased())"
            guard let vault = state.vaults[documentID] else { return server.appending(path: document) }
            return server.appending(path: "vaults/\(vault.uuidString.lowercased())/\(document)")
        }
    }
}

/// The document on screen, as the system sees it: one activity per open document that tells Siri and
/// Apple Intelligence which document is visible. Cleared when the document closes, is deleted, or the
/// session ends or moves to another server.
@MainActor @Observable
public final class DocumentActivityTracker {
    public static let shared = DocumentActivityTracker(locator: .shared)

    public private(set) var current: DocumentActivityPayload?
    private var title: String?
    private var server: URL?
    @ObservationIgnored private let locator: DocumentLocator

    public init(locator: DocumentLocator) {
        self.locator = locator
    }

    public func sessionChanged(to session: SessionRestoration) {
        let next: URL? = if case let .signedIn(url) = session { url } else { nil }
        guard next != server else { return }
        server = next
        clear()
        locator.reset(server: next)
    }

    public func show(_ summary: DocumentSummary, vaultID: UUID) {
        guard let server else { return }
        locator.record(documentID: summary.id, vaultID: vaultID)
        current = DocumentActivityPayload(server: server, vaultID: vaultID, documentID: summary.id)
        title = summary.title
    }

    /// Called when a document's view goes away; a document opened since then stays current.
    public func hide(documentID: Document.ID) {
        guard isCurrent(documentID) else { return }
        clear()
    }

    public func documentRemoved(_ documentID: Document.ID) {
        hide(documentID: documentID)
        locator.forget(documentID: documentID)
    }

    public func isCurrent(_ documentID: Document.ID) -> Bool {
        current?.documentID == documentID
    }

    /// Writes the current document into the scene's activity. The activity stays out of Spotlight and
    /// public indexing; only the app's own scenes and the system's on-screen awareness read it.
    public func configure(_ activity: NSUserActivity) {
        activity.isEligibleForSearch = false
        activity.isEligibleForPublicIndexing = false
        #if os(iOS)
            activity.isEligibleForPrediction = false
        #endif
        activity.isEligibleForHandoff = false
        activity.title = current == nil ? nil : title
        activity.targetContentIdentifier = current?.documentID.uuidString.lowercased()
        activity.requiredUserInfoKeys = current == nil ? [] : DocumentActivityPayload.requiredKeys
        activity.userInfo = current?.userInfo ?? [:]
        if #available(iOS 26, macOS 26, *) {
            activity.appEntityIdentifier = current.map {
                EntityIdentifier(for: DocumentEntity.self, identifier: $0.documentID)
            }
        }
        activity.needsSave = true
    }

    private func clear() {
        current = nil
        title = nil
    }
}
