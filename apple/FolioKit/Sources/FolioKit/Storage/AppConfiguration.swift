import Foundation

/// Identifiers that differ between the official build and a self-build. The build writes them into each
/// target's Info.plist from `Config/Folio.xcconfig`; without those keys (SwiftPM, unit tests) the official
/// values apply.
public enum AppConfiguration {
    public static let officialAppGroup = "group.com.cedricziel.folio"
    public static let officialCallbackScheme = "com.cedricziel.folio"

    public static var appGroup: String {
        appGroup(in: Bundle.main.infoDictionary)
    }

    public static var callbackScheme: String {
        callbackScheme(in: Bundle.main.infoDictionary)
    }

    public static func appGroup(in info: [String: Any]?) -> String {
        value("FolioAppGroup", in: info) ?? officialAppGroup
    }

    public static func callbackScheme(in info: [String: Any]?) -> String {
        value("FolioCallbackScheme", in: info) ?? officialCallbackScheme
    }

    public static func redirectURI(scheme: String) -> String {
        "\(scheme):/oauth/callback"
    }

    private static func value(_ key: String, in info: [String: Any]?) -> String? {
        guard let text = (info?[key] as? String)?.trimmingCharacters(in: .whitespaces), !text.isEmpty else {
            return nil
        }
        return text
    }
}
