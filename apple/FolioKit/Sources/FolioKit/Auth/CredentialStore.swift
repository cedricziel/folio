import Foundation
import Security

public protocol CredentialStore: Sendable {
    func load() throws -> OAuthCredentials?
    /// Replaces the stored credentials in one write, so a rotated refresh token never exists half-saved.
    func save(_ credentials: OAuthCredentials) throws
    func clear() throws
}

public enum CredentialStoreError: Error, Equatable, LocalizedError {
    case keychain(OSStatus)
    case unreadableCredentials

    public var errorDescription: String? {
        switch self {
        case .keychain:
            "Couldn't access your saved sign-in. Try again."
        case .unreadableCredentials:
            "Your saved sign-in couldn't be read. Sign in again."
        }
    }
}

struct KeychainOperations: Sendable {
    var read: @Sendable (CFDictionary) -> (OSStatus, Data?)
    var update: @Sendable (CFDictionary, CFDictionary) -> OSStatus
    var add: @Sendable (CFDictionary) -> OSStatus
    var delete: @Sendable (CFDictionary) -> OSStatus

    static let system = KeychainOperations(
        read: { query in
            var result: CFTypeRef?
            let status = SecItemCopyMatching(query, &result)
            return (status, result as? Data)
        },
        update: { SecItemUpdate($0, $1) },
        add: { SecItemAdd($0, nil) },
        delete: { SecItemDelete($0) }
    )
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
    private var operations = KeychainOperations.system

    public init(
        service: String = "com.cedricziel.folio.credentials",
        accessGroup: String? = SharedConfiguration.keychainAccessGroup,
        dataProtection: Bool = true
    ) {
        self.service = service
        self.accessGroup = accessGroup
        self.dataProtection = dataProtection
    }

    init(operations: KeychainOperations) {
        self.init(service: "test", accessGroup: nil)
        self.operations = operations
    }

    public func load() throws -> OAuthCredentials? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        let (status, data) = operations.read(query as CFDictionary)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw CredentialStoreError.keychain(status) }
        guard let data, let credentials = try? JSONDecoder().decode(OAuthCredentials.self, from: data) else {
            throw CredentialStoreError.unreadableCredentials
        }
        return credentials
    }

    public func save(_ credentials: OAuthCredentials) throws {
        let data = try JSONEncoder().encode(credentials)
        let update = [kSecValueData as String: data]
        var status = operations.update(baseQuery as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            var attributes = baseQuery
            attributes[kSecValueData as String] = data
            attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
            status = operations.add(attributes as CFDictionary)
        }
        guard status == errSecSuccess else { throw CredentialStoreError.keychain(status) }
    }

    public func clear() throws {
        let status = operations.delete(baseQuery as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw CredentialStoreError.keychain(status)
        }
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
