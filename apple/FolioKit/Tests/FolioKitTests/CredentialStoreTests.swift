import Foundation
import Security
import Testing

@testable import FolioKit

struct KeychainCredentialFailureTests {
    private let credentials = OAuthCredentials(
        serverURL: URL(string: "https://archive.example")!, clientID: "test",
        tokenEndpoint: URL(string: "https://archive.example/token")!, revocationEndpoint: nil,
        tokens: OAuthTokens(accessToken: "access", refreshToken: "refresh", expiresAt: .distantFuture)
    )

    private func operations(
        readStatus: OSStatus = errSecSuccess, data: Data? = nil,
        updateStatus: OSStatus = errSecSuccess, addStatus: OSStatus = errSecSuccess,
        deleteStatus: OSStatus = errSecSuccess
    ) -> KeychainOperations {
        KeychainOperations(
            read: { _ in (readStatus, data) }, update: { _, _ in updateStatus },
            add: { _ in addStatus }, delete: { _ in deleteStatus }
        )
    }

    @Test func onlyMissingKeychainItemsMeanNoCredentials() throws {
        let store = KeychainCredentialStore(operations: operations(readStatus: errSecItemNotFound))
        #expect(try store.load() == nil)
    }

    @Test(arguments: [errSecInteractionNotAllowed, errSecNotAvailable, errSecAuthFailed])
    func failedReadsRemainErrors(status: OSStatus) {
        let store = KeychainCredentialStore(operations: operations(readStatus: status))
        #expect(throws: CredentialStoreError.keychain(status)) { try store.load() }
    }

    @Test func malformedCredentialsDoNotLookLikeSignOut() {
        let store = KeychainCredentialStore(operations: operations(data: Data("invalid".utf8)))
        #expect(throws: CredentialStoreError.unreadableCredentials) { try store.load() }
    }

    @Test func successfulReadsDecodeCredentials() throws {
        let store = KeychainCredentialStore(operations: operations(data: try JSONEncoder().encode(credentials)))
        #expect(try store.load() == credentials)
    }

    @Test func failedRotationWriteIsReported() {
        let store = KeychainCredentialStore(operations: operations(updateStatus: errSecInteractionNotAllowed))
        #expect(throws: CredentialStoreError.keychain(errSecInteractionNotAllowed)) {
            try store.save(credentials)
        }
    }

    @Test func failedInitialSignInWriteIsReported() {
        let store = KeychainCredentialStore(
            operations: operations(updateStatus: errSecItemNotFound, addStatus: errSecNotAvailable))
        #expect(throws: CredentialStoreError.keychain(errSecNotAvailable)) { try store.save(credentials) }
    }

    @Test func failedDeletionIsReported() {
        let store = KeychainCredentialStore(operations: operations(deleteStatus: errSecInteractionNotAllowed))
        #expect(throws: CredentialStoreError.keychain(errSecInteractionNotAllowed)) { try store.clear() }
    }

    @Test func deletingMissingCredentialsSucceeds() throws {
        let store = KeychainCredentialStore(operations: operations(deleteStatus: errSecItemNotFound))
        try store.clear()
    }
}

private final class MemoryKeychain: @unchecked Sendable {
    private let lock = NSLock()
    private var data: Data?

    var operations: KeychainOperations {
        KeychainOperations(
            read: { _ in self.lock.withLock { (self.data == nil ? errSecItemNotFound : errSecSuccess, self.data) } },
            update: { _, attributes in
                self.lock.withLock {
                    guard self.data != nil else { return errSecItemNotFound }
                    self.data = (attributes as NSDictionary)[kSecValueData] as? Data
                    return errSecSuccess
                }
            },
            add: { attributes in
                self.lock.withLock {
                    self.data = (attributes as NSDictionary)[kSecValueData] as? Data
                    return errSecSuccess
                }
            },
            delete: { _ in
                self.lock.withLock {
                    self.data = nil; return errSecSuccess
                }
            }
        )
    }
}

extension KeychainCredentialFailureTests {
    @Test func keychainStoreRoundTripsAndReplacesTokens() throws {
        let store = KeychainCredentialStore(operations: MemoryKeychain().operations)
        try store.save(credentials)
        #expect(try store.load() == credentials)

        var rotated = credentials
        rotated.tokens = OAuthTokens(accessToken: "rotated", refreshToken: "next", expiresAt: .distantFuture)
        try store.save(rotated)
        #expect(try store.load() == rotated)
        try store.clear()
        #expect(try store.load() == nil)
    }
}
