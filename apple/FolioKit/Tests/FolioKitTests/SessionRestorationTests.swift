import Foundation
import Testing

@testable import FolioKit

struct SessionRestorationTests {
    private let server = URL(string: "https://archive.example.test")!

    @Test func missingCredentialsShowOnboardingEvenWithRememberedServer() {
        #expect(SessionRestoration.restore(from: InMemoryCredentialStore(), server: server) == .signedOut)
    }

    @Test func expiredSessionRetainsItsServer() {
        let store = InMemoryCredentialStore(credentials(tokens: .signedOut))
        #expect(SessionRestoration.restore(from: store, server: nil) == .expired(server))
    }

    @Test func interruptedRefreshRetainsServerAndRequestsReauthentication() {
        var saved = credentials(
            tokens: OAuthTokens(accessToken: "access", refreshToken: "refresh", expiresAt: .distantPast))
        saved.refreshPending = true
        #expect(SessionRestoration.restore(from: InMemoryCredentialStore(saved), server: nil) == .interrupted(server))
    }

    @Test func expiredAccessTokenStillRestoresSessionForRefresh() {
        let store = InMemoryCredentialStore(
            credentials(tokens: OAuthTokens(accessToken: "access", refreshToken: "refresh", expiresAt: .distantPast)))
        #expect(SessionRestoration.restore(from: store, server: nil) == .signedIn(server))
    }

    @Test func failedReadRetainsServerAndRecoversOnRetry() {
        let store = RecoveringStore(
            credentials: credentials(
                tokens: OAuthTokens(accessToken: "access", refreshToken: "refresh", expiresAt: .distantFuture)))
        #expect(
            SessionRestoration.restore(from: store, server: server)
                == .unavailable(
                    server: server, message: CredentialStoreError.keychain(-25308).localizedDescription))
        store.recover()
        #expect(SessionRestoration.restore(from: store, server: server) == .signedIn(server))
    }

    private func credentials(tokens: OAuthTokens) -> OAuthCredentials {
        OAuthCredentials(
            serverURL: server, clientID: "client", tokenEndpoint: server.appending(path: "oauth/token"),
            revocationEndpoint: nil, tokens: tokens)
    }
}

private final class RecoveringStore: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var unavailable = true
    private let credentials: OAuthCredentials

    init(credentials: OAuthCredentials) { self.credentials = credentials }

    func recover() { lock.withLock { unavailable = false } }

    func load() throws -> OAuthCredentials? {
        try lock.withLock {
            if unavailable { throw CredentialStoreError.keychain(-25308) }
            return credentials
        }
    }

    func save(_ credentials: OAuthCredentials) {}
    func clear() {}
}
