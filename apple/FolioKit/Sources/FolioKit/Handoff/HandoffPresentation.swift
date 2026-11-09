import Foundation

/// What the app shows for a decided continuation. Views switch over this and nothing else.
public enum HandoffPresentation: Equatable, Sendable {
    case navigate(HandoffRoute)
    case alert(title: String, message: String, browserURL: URL)
    case browser(URL)
    case prefillSignIn(URL)
    case nothing

    public enum Action: Equatable, Sendable {
        case openInBrowser(URL)
        case cancel

        public var title: String {
            switch self {
            case .openInBrowser: "Open in Browser"
            case .cancel: "Cancel"
            }
        }
    }

    public static func from(_ decision: HandoffDecision, currentServer: URL?) -> Self {
        switch decision {
        case .hold, .ignore:
            return .nothing
        case let .open(route):
            return .navigate(route)
        case let .signIn(prefill, _):
            return .prefillSignIn(prefill)
        case let .openInBrowser(url):
            return .browser(url)
        case let .otherServer(host, web):
            let noun =
                switch HandoffLink.parse(web)?.target {
                case .library: "vault"
                case .savedView: "saved view"
                case .document, nil: "document"
                }
            var message = "This \(noun) is on \(host)."
            if let current = currentServer.flatMap(ServerIdentity.init)?.displayHost {
                message += " You're signed in to \(current)."
            }
            return .alert(title: "On another server", message: message, browserURL: web)
        }
    }

    /// The buttons an alert offers, in order.
    public var actions: [Action] {
        if case let .alert(_, _, url) = self { [.openInBrowser(url), .cancel] } else { [] }
    }
}
