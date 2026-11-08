import Foundation

/// The activity types Folio advertises and accepts, declared in the app's `NSUserActivityTypes`.
public enum HandoffActivityType {
    public static let document = DocumentActivityPayload.activityType
    public static let library = "com.cedricziel.folio.library"
    public static let savedView = "com.cedricziel.folio.saved-view"
    public static let all = [document, library, savedView]
}

public enum HandoffPane: String, Equatable, Sendable {
    case discussion
}

public enum HandoffList: String, Equatable, Sendable {
    case inbox
    case all
}

public enum HandoffTarget: Equatable, Sendable {
    /// `page` is the 1-based reading position, when the sender's preview has pages.
    case document(Document.ID, pane: HandoffPane? = nil, page: Int? = nil)
    case library(HandoffList)
    case savedView(UUID)
}

/// What one Folio screen hands to another device: the sender's server, the vault and the target, as
/// identifiers only. The document form uses exactly the keys of `DocumentActivityPayload`, plus an
/// optional `pane`, so either decoder reads the other's activities.
public struct HandoffPayload: Equatable, Sendable {
    public static let version = DocumentActivityPayload.version

    public let server: URL
    public let vaultID: UUID
    public let target: HandoffTarget

    /// Keeps only the server's canonical form, so user info, a query or a fragment in the session's URL
    /// never travels to another device.
    public init(server: URL, vaultID: UUID, target: HandoffTarget) {
        self.server = ServerIdentity(server)?.url ?? server
        self.vaultID = vaultID
        self.target = target
    }

    public init(_ document: DocumentActivityPayload) {
        self.init(server: document.server, vaultID: document.vaultID, target: .document(document.documentID))
    }

    public var activityType: String {
        switch target {
        case .document: HandoffActivityType.document
        case .library: HandoffActivityType.library
        case .savedView: HandoffActivityType.savedView
        }
    }

    public var requiredUserInfoKeys: Set<String> {
        let target =
            switch self.target {
            case .document: "document"
            case .library: "list"
            case .savedView: "savedView"
            }
        return ["version", "server", "vault", target]
    }

    public var userInfo: [AnyHashable: Any] {
        var info: [AnyHashable: Any] = [
            "version": Self.version,
            "server": server.absoluteString,
            "vault": vaultID.uuidString.lowercased(),
        ]
        switch target {
        case let .document(id, pane, page):
            info["document"] = id.uuidString.lowercased()
            if let pane { info["pane"] = pane.rawValue }
            if let page { info["page"] = page }
        case let .library(list):
            info["list"] = list.rawValue
        case let .savedView(id):
            info["savedView"] = id.uuidString.lowercased()
        }
        return info
    }

    /// Fails for an unknown type, a missing key, an invalid UUID, a server that is not http(s) with a
    /// host, or another `version`. Unknown keys, an unknown `pane` and a `page` that is not an integer
    /// are ignored; `page` is optional within version 1.
    public init?(activityType: String, userInfo: [AnyHashable: Any]?) {
        guard let userInfo, Self.integer(userInfo["version"]) == Self.version,
            let address = userInfo["server"] as? String, let server = URL(string: address),
            ServerIdentity(server) != nil,
            let vault = Self.uuid(userInfo["vault"])
        else { return nil }
        let target: HandoffTarget
        switch activityType {
        case HandoffActivityType.document:
            guard let document = Self.uuid(userInfo["document"]) else { return nil }
            target = .document(
                document, pane: (userInfo["pane"] as? String).flatMap(HandoffPane.init(rawValue:)),
                page: Self.integer(userInfo["page"]))
        case HandoffActivityType.library:
            guard let list = (userInfo["list"] as? String).flatMap(HandoffList.init(rawValue:)) else { return nil }
            target = .library(list)
        case HandoffActivityType.savedView:
            guard let view = Self.uuid(userInfo["savedView"]) else { return nil }
            target = .savedView(view)
        default:
            return nil
        }
        self.init(server: server, vaultID: vault, target: target)
    }

    /// The same document as #353's on-screen activity, without the Handoff extras.
    public var documentActivity: DocumentActivityPayload? {
        guard case let .document(id, _, _) = target else { return nil }
        return DocumentActivityPayload(server: server, vaultID: vaultID, documentID: id)
    }

    /// An integer, and only an integer: user info arrives as NSNumber, which bridges Booleans and
    /// floating-point values to `Int` too.
    static func integer(_ value: Any?) -> Int? {
        guard let value else { return nil }
        #if canImport(ObjectiveC)
            // Swift numbers and Booleans bridge to NSNumber here, so this one check covers both.
            guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
                !CFNumberIsFloatType(number as CFNumber)
            else { return nil }
            return number.intValue
        #else
            if value is Bool || value is Double || value is Float { return nil }
            if let number = value as? NSNumber, ["c", "B", "f", "d"].contains(String(cString: number.objCType)) {
                return nil
            }
            return value as? Int
        #endif
    }

    private static func uuid(_ value: Any?) -> UUID? {
        (value as? String).flatMap(UUID.init(uuidString:))
    }
}
