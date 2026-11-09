import Foundation

public protocol ServerConfigStore: Sendable {
    func load() -> URL?
    func save(_ url: URL?)
}

public struct UserDefaultsServerConfigStore: ServerConfigStore {
    /// Shared with the share extension through an App Group.
    public static var appGroup: String { AppConfiguration.appGroup }

    private nonisolated(unsafe) let defaults: UserDefaults
    private let key = "serverURL"

    public init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    /// The app-group suite when the entitlement is present, standard defaults otherwise.
    public static func shared() -> UserDefaultsServerConfigStore {
        UserDefaultsServerConfigStore(defaults: UserDefaults(suiteName: appGroup) ?? .standard)
    }

    public func load() -> URL? {
        defaults.string(forKey: key).flatMap(URL.init(string:))
    }

    public func save(_ url: URL?) {
        defaults.set(url?.absoluteString, forKey: key)
    }
}

public enum ServerAddress {
    /// Accepts "host:3000", "http://host" or a full URL. Bare hosts default to https.
    public static func parse(_ input: String) throws -> URL {
        var text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw FolioError.invalidServerURL }
        if !text.contains("://") {
            text = "https://" + text
        }

        guard var components = URLComponents(string: text),
            let scheme = components.scheme?.lowercased(), ["http", "https"].contains(scheme),
            let host = components.host, !host.isEmpty
        else { throw FolioError.invalidServerURL }

        components.query = nil
        components.fragment = nil
        while components.path.hasSuffix("/") {
            components.path.removeLast()
        }
        guard let url = components.url else { throw FolioError.invalidServerURL }
        return url
    }
}
