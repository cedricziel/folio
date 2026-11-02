import Foundation
import LocalAuthentication
import Security

/// Persists an installation UUID without replacing an identity created by another process.
public protocol InstallationIDStore: Sendable {
    /// Returns nil when absent and throws when storage cannot be read.
    func load() throws -> UUID?
    /// Returns the stored UUID, preserving an existing value if another process wins the insertion.
    func insertIfAbsent(_ candidate: UUID) throws -> UUID
}

/// Stores the installation UUID in the Keychain access group shared with Folio extensions.
public struct KeychainInstallationIDStore: InstallationIDStore {
    private let accessGroup: String?

    public init(accessGroup: String? = SharedConfiguration.keychainAccessGroup) {
        self.accessGroup = accessGroup
    }

    public func load() throws -> UUID? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        query[kSecUseAuthenticationContext as String] = authenticationContext
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw failure(status) }
        guard let data = result as? Data, let text = String(data: data, encoding: .utf8),
            let identifier = UUID(uuidString: text)
        else { throw failure(errSecDecode) }
        return identifier
    }

    public func insertIfAbsent(_ candidate: UUID) throws -> UUID {
        var attributes = baseQuery
        attributes[kSecValueData as String] = Data(candidate.uuidString.lowercased().utf8)
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        attributes[kSecUseAuthenticationContext as String] = authenticationContext
        let status = SecItemAdd(attributes as CFDictionary, nil)
        if status == errSecSuccess { return candidate }
        if status == errSecDuplicateItem, let existing = try load() { return existing }
        throw failure(status)
    }

    private var baseQuery: [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.cedricziel.folio.telemetry",
            kSecAttrAccount as String: "installationID",
            kSecAttrSynchronizable as String: false,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        return query
    }

    private var authenticationContext: LAContext {
        let context = LAContext()
        context.interactionNotAllowed = true
        return context
    }

    private func failure(_ status: OSStatus) -> NSError {
        NSError(domain: NSOSStatusErrorDomain, code: Int(status))
    }
}

final class InstallationIdentity: @unchecked Sendable {
    private let lock = NSLock()
    private let store: any InstallationIDStore
    private var cached: UUID?

    init(store: any InstallationIDStore) {
        self.store = store
    }

    func resolve() -> UUID? {
        lock.withLock {
            if let cached { return cached }
            do {
                let identifier: UUID
                if let existing = try store.load() {
                    identifier = existing
                } else {
                    identifier = try store.insertIfAbsent(UUID())
                }
                cached = identifier
                return identifier
            } catch {
                // A locked Keychain must not turn the same installation into a new identity.
                return nil
            }
        }
    }
}
