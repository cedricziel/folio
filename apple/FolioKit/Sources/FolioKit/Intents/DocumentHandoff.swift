import Foundation
import Observation

/// Continues a document another device handed off. The activity carries only identifiers, so this device
/// opens it with its own sign-in, and only when that sign-in is for the server the activity names.
@MainActor @Observable
public final class DocumentHandoff {
    public static let shared = DocumentHandoff(router: .shared)

    /// A document waiting for this device to finish restoring, or for the person to sign in.
    public private(set) var pending: DocumentActivityPayload?
    /// Why a handed-off document did not open, in words the person can act on.
    public private(set) var problem: String?
    @ObservationIgnored private let router: IntentRouter
    @ObservationIgnored private var routed: IntentRouter.Request?

    public init(router: IntentRouter) {
        self.router = router
    }

    public func receive(_ userInfo: [AnyHashable: Any]?, session: SessionRestoration) {
        guard let payload = DocumentActivityPayload(userInfo: userInfo) else {
            pending = nil
            problem = "This document can't be continued here. Update Folio on both devices and try again."
            return
        }
        pending = payload
        problem = nil
        resolve(session)
    }

    public func sessionChanged(to session: SessionRestoration) {
        let signedIn = if case .signedIn = session { true } else { false }
        // A document the workspace never picked up must not open in whichever session comes next.
        if !signedIn, let routed, router.request == routed {
            router.clear()
        }
        resolve(session)
    }

    public func dismissProblem() {
        problem = nil
    }

    private func resolve(_ session: SessionRestoration) {
        guard let payload = pending else { return }
        let host = payload.server.host() ?? payload.server.absoluteString
        switch session {
        case .restoring:
            return
        case let .signedIn(server) where Self.sameServer(server, payload.server):
            pending = nil
            problem = nil
            let request = IntentRouter.Request.document(payload.documentID, vaultID: payload.vaultID)
            routed = request
            router.route(request)
        case let .signedIn(server):
            pending = nil
            problem =
                "This document is on \(host), but Folio here is signed in to \(server.host() ?? server.absoluteString)."
        case .signedOut, .expired, .interrupted, .unavailable:
            problem = "Sign in to \(host) to continue reading this document."
        }
    }

    static func sameServer(_ lhs: URL, _ rhs: URL) -> Bool {
        key(lhs) == key(rhs)
    }

    private static func key(_ url: URL) -> String? {
        guard var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
        components.scheme = components.scheme?.lowercased()
        components.host = components.host?.lowercased()
        while components.path.hasSuffix("/") { components.path.removeLast() }
        components.query = nil
        components.fragment = nil
        return components.string
    }
}
