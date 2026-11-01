import Foundation
import Security

/// Bearer token in the keychain, shared by the app and its extensions through a keychain access group.
/// The API has no auth yet; this is where the OAuth token will live.
public struct KeychainTokenStore: BearerTokenProvider {
    private let service = "com.cedricziel.folio.token"
    private let account = "bearer"
    private let accessGroup: String?

    /// `accessGroup` is `<team prefix>com.cedricziel.folio`; targets read the prefix from their Info.plist.
    public init(accessGroup: String? = SharedConfiguration.keychainAccessGroup) {
        self.accessGroup = accessGroup
    }

    public func token() async -> String? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
            let data = result as? Data
        else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    public func save(_ token: String?) {
        SecItemDelete(baseQuery as CFDictionary)
        guard let token else { return }
        var attributes = baseQuery
        attributes[kSecValueData as String] = Data(token.utf8)
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        SecItemAdd(attributes as CFDictionary, nil)
    }

    private var baseQuery: [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let accessGroup {
            query[kSecAttrAccessGroup as String] = accessGroup
        }
        return query
    }
}

/// What the app and every extension agree on: where the server is and how to authenticate.
public enum SharedConfiguration {
    public static let appGroup = UserDefaultsServerConfigStore.appGroup

    /// Info.plist key holding `$(AppIdentifierPrefix)com.cedricziel.folio`, which is only known at build time.
    public static var keychainAccessGroup: String? {
        Bundle.main.object(forInfoDictionaryKey: "FolioKeychainGroup") as? String
    }

    public static func serverURL() -> URL? {
        UserDefaultsServerConfigStore.shared().load()
    }

    public static func makeService(session: URLSession = .shared) -> LiveFolioService? {
        serverURL().map {
            LiveFolioService(baseURL: $0, tokenProvider: KeychainTokenStore(), session: session)
        }
    }

    public static var container: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
    }
}
