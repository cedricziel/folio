import Foundation

/// Where a tap on a widget takes the app. The app registers ``scheme`` only when widgets are built in; it
/// follows the build's bundle identifier, so a self-build's links never open the official app.
public enum WidgetLink: Equatable, Sendable {
    case inbox
    case scan
    case document(Document.ID, vaultID: UUID)

    public static var scheme: String { "\(AppConfiguration.callbackScheme).widgets" }

    public var url: URL {
        switch self {
        case .inbox: URL(string: "\(Self.scheme)://inbox")!
        case .scan: URL(string: "\(Self.scheme)://scan")!
        case let .document(id, vaultID):
            URL(string: "\(Self.scheme)://document/\(vaultID.uuidString.lowercased())/\(id.uuidString.lowercased())")!
        }
    }

    public init?(url: URL) {
        // Schemes are case-insensitive, and a self-build bundle id may contain capitals.
        guard url.scheme?.lowercased() == Self.scheme.lowercased() else { return nil }
        let path = url.pathComponents.filter { $0 != "/" }
        switch (url.host(), path.count) {
        case ("inbox", 0): self = .inbox
        case ("scan", 0): self = .scan
        case ("document", 2):
            guard let vaultID = UUID(uuidString: path[0]), let id = UUID(uuidString: path[1]) else { return nil }
            self = .document(id, vaultID: vaultID)
        default: return nil
        }
    }

    public var request: IntentRouter.Request {
        switch self {
        case .inbox: .inbox
        case .scan: .scan
        case let .document(id, vaultID): .document(id, vaultID: vaultID)
        }
    }
}
