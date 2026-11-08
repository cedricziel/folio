import AppIntents
import Foundation
import Observation

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

/// The document on screen in one scene, as the system sees it: one activity per open document that tells
/// Siri and Apple Intelligence which document is visible. Each window keeps its own tracker.
///
/// The activity follows the session the way the Spotlight index does: an interrupted or restoring session
/// withdraws it and the same server coming back publishes it again; sign-out, an expired session, another
/// server or an interactive sign-in forget it.
@MainActor @Observable
public final class DocumentActivityTracker {
    public private(set) var current: DocumentActivityPayload?
    @ObservationIgnored public let isEligibleForHandoff: Bool
    @ObservationIgnored private var shown: (summary: DocumentSummary, vaultID: UUID)?
    @ObservationIgnored private var server: URL?
    @ObservationIgnored private var suspended = true

    /// - Parameter isEligibleForHandoff: Whether other devices may continue the open document.
    public init(isEligibleForHandoff: Bool = false) {
        self.isEligibleForHandoff = isEligibleForHandoff
    }

    public func sessionChanged(to session: SessionRestoration) {
        switch session {
        case let .signedIn(url):
            if url != server {
                forget()
                server = url
            }
            suspended = false
            publish()
        case .signedOut, .expired:
            forget()
            server = nil
        case .restoring, .interrupted, .unavailable:
            suspended = true
            current = nil
        }
    }

    /// An interactive sign-in may be another account on the same server, so nothing carries over.
    public func signedIn(to url: URL) {
        forget()
        server = url
        suspended = false
    }

    public func show(_ summary: DocumentSummary, vaultID: UUID) {
        shown = (summary, vaultID)
        publish()
    }

    /// Called when a document's view goes away; a document opened since then stays current.
    public func hide(documentID: Document.ID) {
        guard shown?.summary.id == documentID else { return }
        shown = nil
        current = nil
    }

    public func documentRemoved(_ documentID: Document.ID) {
        hide(documentID: documentID)
    }

    public func isCurrent(_ documentID: Document.ID) -> Bool {
        current?.documentID == documentID
    }

    /// Writes the current document into the scene's activity. It carries identifiers only, no title, and
    /// stays out of Spotlight, public indexing and Siri suggestions.
    public func configure(_ activity: NSUserActivity) {
        activity.isEligibleForSearch = false
        activity.isEligibleForPublicIndexing = false
        #if os(iOS)
            activity.isEligibleForPrediction = false
        #endif
        activity.isEligibleForHandoff = isEligibleForHandoff && current != nil
        activity.title = nil
        activity.targetContentIdentifier = current?.documentID.uuidString.lowercased()
        activity.requiredUserInfoKeys = current == nil ? [] : DocumentActivityPayload.requiredKeys
        activity.userInfo = current?.userInfo ?? [:]
        if #available(iOS 26, macOS 26, *) {
            activity.appEntityIdentifier = current.map {
                EntityIdentifier(
                    for: DocumentEntity.self,
                    identifier: DocumentEntityID(documentID: $0.documentID, vaultID: $0.vaultID))
            }
        }
        activity.needsSave = true
    }

    private func publish() {
        guard !suspended, let server, let shown else {
            current = nil
            return
        }
        current = DocumentActivityPayload(server: server, vaultID: shown.vaultID, documentID: shown.summary.id)
    }

    private func forget() {
        shown = nil
        current = nil
    }
}
