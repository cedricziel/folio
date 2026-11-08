import Foundation
import Testing

@testable import FolioKit

private final class RecoveryURLProtocol: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var failure: URLError.Code?
    nonisolated(unsafe) private static var count = 0

    static func reset(failure: URLError.Code? = nil) {
        lock.withLock {
            Self.failure = failure; count = 0
        }
    }

    static var requests: Int { lock.withLock { count } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let failing = Self.lock.withLock {
            Self.count += 1; return Self.failure
        }
        if let failing {
            client?.urlProtocol(self, didFailWithError: URLError(failing))
            return
        }
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(#"{"access_token":"new","refresh_token":"r2","expires_in":3600}"#.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private final class FailingRotationStore: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var credentials: OAuthCredentials?
    private var failReplacement = true

    init(_ credentials: OAuthCredentials) { self.credentials = credentials }
    func load() -> OAuthCredentials? { lock.withLock { credentials } }
    func save(_ credentials: OAuthCredentials) throws {
        try lock.withLock {
            if credentials.tokens.accessToken == "new", failReplacement {
                failReplacement = false
                throw CocoaError(.fileWriteNoPermission)
            }
            self.credentials = credentials
        }
    }
    func clear() { lock.withLock { credentials = nil } }
}

@Suite(.serialized, .timeLimit(.minutes(1))) struct OAuthRecoveryTests {
    private let clock = Date(timeIntervalSince1970: 1_800_000_000)

    private func credentials() -> OAuthCredentials {
        OAuthCredentials(
            serverURL: URL(string: "https://folio.example.com")!, clientID: "client",
            tokenEndpoint: URL(string: "https://folio.example.com/oauth/token")!, revocationEndpoint: nil,
            tokens: OAuthTokens(accessToken: "old", refreshToken: "r1", expiresAt: clock.addingTimeInterval(-1)))
    }

    private func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [RecoveryURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test func failedRotationSavePreventsReplayAndCanRecoverWithoutAnotherRequest() async throws {
        RecoveryURLProtocol.reset()
        let directory = try directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = FailingRotationStore(credentials())
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let first = OAuthSession(
            store: store, session: transport, now: { clock },
            refreshLock: OAuthRefreshLock(url: directory.appending(path: "refresh.lock")))
        let second = OAuthSession(
            store: store, session: transport, now: { clock },
            refreshLock: OAuthRefreshLock(url: directory.appending(path: "refresh.lock")))

        await #expect(throws: CocoaError.self) { try await first.token() }
        #expect(store.load()?.refreshPending == true)
        await #expect(throws: FolioError.signInRequired) { try await second.token() }
        #expect(try await first.token() == "new")
        #expect(store.load()?.refreshPending != true)
        #expect(store.load()?.tokens.refreshToken == "r2")
        #expect(RecoveryURLProtocol.requests == 1)
    }

    @Test func pendingReplacementCannotRestoreCredentialsClearedByAnotherProcess() async throws {
        RecoveryURLProtocol.reset()
        let store = FailingRotationStore(credentials())
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: transport, now: { clock })
        await #expect(throws: CocoaError.self) { try await oauth.token() }
        store.clear()

        #expect(try await oauth.token() == nil)
        #expect(store.load() == nil)
        #expect(RecoveryURLProtocol.requests == 1)
    }

    @Test func pendingReplacementCannotOverwriteAnotherSignIn() async throws {
        RecoveryURLProtocol.reset()
        let store = FailingRotationStore(credentials())
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: transport, now: { clock })
        await #expect(throws: CocoaError.self) { try await oauth.token() }
        var replacement = credentials()
        replacement.tokens = OAuthTokens(
            accessToken: "another-account", refreshToken: "another-refresh",
            expiresAt: clock.addingTimeInterval(3600))
        try store.save(replacement)

        #expect(try await oauth.token() == "another-account")
        #expect(store.load() == replacement)
        #expect(RecoveryURLProtocol.requests == 1)
    }

    @Test func expiredAccessWithoutRefreshTokenPersistsReauthenticationState() async throws {
        RecoveryURLProtocol.reset()
        var expired = credentials()
        expired.tokens.refreshToken = nil
        let store = InMemoryCredentialStore(expired)
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: transport, now: { clock })

        #expect(try await oauth.token() == nil)
        #expect(store.load()?.isSignedIn == false)
        #expect(store.load()?.serverURL == expired.serverURL)
        #expect(RecoveryURLProtocol.requests == 0)
    }

    @Test func independentSessionsShareOneRotation() async throws {
        RecoveryURLProtocol.reset()
        let directory = try directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = InMemoryCredentialStore(credentials())
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let first = OAuthSession(
            store: store, session: transport, now: { clock },
            refreshLock: OAuthRefreshLock(url: directory.appending(path: "refresh.lock")))
        let second = OAuthSession(
            store: store, session: transport, now: { clock },
            refreshLock: OAuthRefreshLock(url: directory.appending(path: "refresh.lock")))

        async let a = first.token()
        async let b = second.token()
        let tokens = try await [a, b]

        #expect(tokens == ["new", "new"])
        #expect(RecoveryURLProtocol.requests == 1)
        #expect(store.load()?.tokens.refreshToken == "r2")
    }

    @Test func offlineRefreshRetainsCredentialsForRetry() async throws {
        RecoveryURLProtocol.reset(failure: .notConnectedToInternet)
        let original = credentials()
        let store = InMemoryCredentialStore(original)
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: transport, now: { clock })

        await #expect(throws: FolioError.unreachable) { try await oauth.token() }
        #expect(store.load() == original)

        RecoveryURLProtocol.reset()
        #expect(try await oauth.token() == "new")
        #expect(store.load()?.refreshPending != true)
        #expect(RecoveryURLProtocol.requests == 1)
    }

    @Test func lostRefreshResponsePreservesIntentAndNeverReplaysConsumedToken() async throws {
        RecoveryURLProtocol.reset(failure: .networkConnectionLost)
        let store = InMemoryCredentialStore(credentials())
        let transport = session()
        defer { transport.invalidateAndCancel() }
        let first = OAuthSession(store: store, session: transport, now: { clock })
        let restarted = OAuthSession(store: store, session: transport, now: { clock })

        await #expect(throws: FolioError.unreachable) { try await first.token() }
        #expect(store.load()?.refreshPending == true)
        #expect(store.load()?.tokens.refreshToken == "r1")
        await #expect(throws: FolioError.signInRequired) { try await restarted.token() }
        #expect(RecoveryURLProtocol.requests == 1)
    }
}
