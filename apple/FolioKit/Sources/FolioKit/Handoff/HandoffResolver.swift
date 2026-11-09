import Foundation

/// A continuation the system delivered: a Handoff activity, a universal link or a Spotlight result.
public enum HandoffIncoming: Equatable, Sendable {
    case payload(HandoffPayload)
    /// A payload that did not decode, or a link to a page that is not a handed-over target.
    case unreadable(webpageURL: URL?)
    case spotlight(DocumentLink)

    public static func activity(type: String, userInfo: [AnyHashable: Any]?, webpageURL: URL?) -> Self {
        HandoffPayload(activityType: type, userInfo: userInfo).map(Self.payload) ?? .unreadable(webpageURL: webpageURL)
    }

    public static func universalLink(_ url: URL) -> Self {
        HandoffLink.parse(url).map(Self.payload) ?? .unreadable(webpageURL: url)
    }
}

/// Where a continuation opens in the workspace.
public struct HandoffRoute: Equatable, Sendable {
    public let vaultID: UUID
    public let target: HandoffTarget

    public init(vaultID: UUID, target: HandoffTarget) {
        self.vaultID = vaultID
        self.target = target
    }
}

public enum HandoffDecision: Equatable, Sendable {
    /// Wait: the session is restoring, or the payload's server needs the user to reconnect.
    case hold
    case open(HandoffRoute)
    case signIn(prefill: URL, then: HandoffRoute)
    /// The app uses another server; offer the sender's web page instead.
    case otherServer(host: String, web: URL)
    case openInBrowser(URL)
    case ignore

    public var route: HandoffRoute? {
        if case let .open(route) = self { route } else { nil }
    }
}

public enum HandoffResolver {
    /// Decides what a continuation does in the current session. Never changes the session itself.
    public static func decide(incoming: HandoffIncoming, session: SessionRestoration) -> HandoffDecision {
        switch incoming {
        case let .unreadable(webpageURL):
            return webpageURL.map(HandoffDecision.openInBrowser) ?? .ignore
        case let .spotlight(link):
            let route = HandoffRoute(vaultID: link.vaultID, target: .document(link.documentID))
            switch session {
            case .signedIn: return .open(route)
            case .signedOut: return .ignore
            case .restoring, .expired, .interrupted, .unavailable: return .hold
            }
        case let .payload(payload):
            let route = HandoffRoute(vaultID: payload.vaultID, target: payload.target)
            let elsewhere: HandoffDecision = {
                guard let web = HandoffLink.webURL(for: payload) else { return .ignore }
                let host = ServerIdentity(payload.server)?.displayHost ?? payload.server.absoluteString
                return .otherServer(host: host, web: web)
            }()
            switch session {
            case .restoring:
                return .hold
            case .signedOut:
                return .signIn(prefill: payload.server, then: route)
            case let .signedIn(url):
                return ServerIdentity.same(url, payload.server) ? .open(route) : elsewhere
            case let .expired(url), let .interrupted(url):
                return ServerIdentity.same(url, payload.server) ? .hold : elsewhere
            case let .unavailable(server, _):
                guard let server, !ServerIdentity.same(server, payload.server) else { return .hold }
                return elsewhere
            }
        }
    }
}
