import Foundation
import Security

public protocol CredentialStore: Sendable {
    func load() -> OAuthCredentials?
    /// Replaces the stored credentials in one write, so a rotated refresh token never exists half-saved.
    func save(_ credentials: OAuthCredentials)
    func clear()
}

public final class InMemoryCredentialStore: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var credentials: OAuthCredentials?

    public init(_ credentials: OAuthCredentials? = nil) {
        self.credentials = credentials
    }

    public func load() -> OAuthCredentials? {
        lock.withLock { credentials }
    }

    public func save(_ credentials: OAuthCredentials) {
        lock.withLock { self.credentials = credentials }
    }

    public func clear() {
        lock.withLock { credentials = nil }
    }
}

/// Credentials as one keychain item in the access group the app and its extensions share.
public struct KeychainCredentialStore: CredentialStore {
    private let service: String
    private let accessGroup: String?
    private let dataProtection: Bool

    public init(
        service: String = "com.cedricziel.folio.credentials",
        accessGroup: String? = SharedConfiguration.keychainAccessGroup,
        dataProtection: Bool = true
    ) {
        self.service = service
        self.accessGroup = accessGroup
        self.dataProtection = dataProtection
    }

    public func load() -> OAuthCredentials? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let data = result as? Data
        else { return nil }
        return try? JSONDecoder().decode(OAuthCredentials.self, from: data)
    }

    public func save(_ credentials: OAuthCredentials) {
        guard let data = try? JSONEncoder().encode(credentials) else { return }
        let update = [kSecValueData as String: data]
        if SecItemUpdate(baseQuery as CFDictionary, update as CFDictionary) == errSecItemNotFound {
            var attributes = baseQuery
            attributes[kSecValueData as String] = data
            attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
            SecItemAdd(attributes as CFDictionary, nil)
        }
    }

    public func clear() {
        SecItemDelete(baseQuery as CFDictionary)
    }

    private var baseQuery: [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: "oauth",
        ]
        if dataProtection {
            query[kSecUseDataProtectionKeychain as String] = true
        }
        if let accessGroup {
            query[kSecAttrAccessGroup as String] = accessGroup
        }
        return query
    }
}

/// Dynamic client registration is per server and survives sign-out, so it is cached apart from the tokens.
public protocol ClientRegistry: Sendable {
    func clientID(for server: URL) -> String?
    func save(clientID: String, for server: URL)
}

public final class InMemoryClientRegistry: ClientRegistry, @unchecked Sendable {
    private let lock = NSLock()
    private var ids: [String: String] = [:]

    public init() {}

    public func clientID(for server: URL) -> String? {
        lock.withLock { ids[server.absoluteString] }
    }

    public func save(clientID: String, for server: URL) {
        lock.withLock { ids[server.absoluteString] = clientID }
    }
}

public struct UserDefaultsClientRegistry: ClientRegistry {
    private nonisolated(unsafe) let defaults: UserDefaults

    public init(defaults: UserDefaults = UserDefaults(suiteName: UserDefaultsServerConfigStore.appGroup) ?? .standard) {
        self.defaults = defaults
    }

    public func clientID(for server: URL) -> String? {
        defaults.dictionary(forKey: "oauthClients")?[server.absoluteString] as? String
    }

    public func save(clientID: String, for server: URL) {
        var all = defaults.dictionary(forKey: "oauthClients") ?? [:]
        all[server.absoluteString] = clientID
        defaults.set(all, forKey: "oauthClients")
    }
}
