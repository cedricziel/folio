import Foundation
import Testing

@testable import FolioKit

private final class CountingTokenURLProtocol: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var count = 0

    static func reset() { lock.withLock { count = 0 } }
    static var requests: Int { lock.withLock { count } }

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lock.withLock { Self.count += 1 }
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(#"{"access_token":"new","refresh_token":"r2","expires_in":3600}"#.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

/// Serves `first` once, then `next`, as if a sign-in replaced the credentials between two reads.
private final class SwitchingStore: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var credentials: OAuthCredentials?
    private var next: OAuthCredentials?

    init(first: OAuthCredentials, next: OAuthCredentials) {
        credentials = first
        self.next = next
    }

    func load() -> OAuthCredentials? {
        lock.withLock {
            defer {
                if let next {
                    credentials = next
                    self.next = nil
                }
            }
            return credentials
        }
    }
    func save(_ credentials: OAuthCredentials) { lock.withLock { self.credentials = credentials } }
    func clear() { lock.withLock { credentials = nil } }
}

/// Fails the first save of a rotated token, leaving it pending in the session.
private final class FailingRotationStore: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var credentials: OAuthCredentials?
    private var failRotation = true

    init(_ credentials: OAuthCredentials) { self.credentials = credentials }
    func load() -> OAuthCredentials? { lock.withLock { credentials } }
    func save(_ credentials: OAuthCredentials) throws {
        try lock.withLock {
            if credentials.tokens.accessToken == "new", failRotation {
                failRotation = false
                throw CocoaError(.fileWriteNoPermission)
            }
            self.credentials = credentials
        }
    }
    func clear() { lock.withLock { credentials = nil } }
}

@Suite(.serialized) struct ServerBoundTokenTests {
    private let signedIn = URL(string: "https://b.example.com")!
    private let remembered = URL(string: "https://a.example.com")!
    private let clock = Date(timeIntervalSince1970: 1_800_000_000)

    private func credentials(for server: URL, prefix: String, expiresIn: TimeInterval) -> OAuthCredentials {
        OAuthCredentials(
            serverURL: server, clientID: "client", tokenEndpoint: server.appending(path: "oauth/token"),
            revocationEndpoint: nil,
            tokens: OAuthTokens(
                accessToken: "\(prefix)-token", refreshToken: "\(prefix)-refresh",
                expiresAt: clock.addingTimeInterval(expiresIn)
            )
        )
    }

    private func store(expiresIn: TimeInterval = 3600) -> InMemoryCredentialStore {
        InMemoryCredentialStore(credentials(for: signedIn, prefix: "b", expiresIn: expiresIn))
    }

    private func transport() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CountingTokenURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    @Test func aTokenIsOnlyHandedToTheServerItWasIssuedFor() async throws {
        let oauth = OAuthSession(store: store(), now: { clock })
        #expect(try await oauth.token(for: signedIn) == "b-token")
        #expect(try await oauth.token(for: remembered) == nil)
        #expect(try await ServerBoundTokenProvider(session: oauth, server: signedIn).token() == "b-token")
        #expect(try await ServerBoundTokenProvider(session: oauth, server: remembered).token() == nil)
    }

    @Test func anotherServerCannotTriggerARefresh() async throws {
        let store = store(expiresIn: -1)
        let saved = store.load()
        let oauth = OAuthSession(store: store, now: { clock })
        #expect(try await oauth.token(for: remembered) == nil)
        #expect(try await oauth.refresh(rejecting: "b-token", for: remembered) == nil)
        let bound = ServerBoundTokenProvider(session: oauth, server: remembered)
        #expect(try await bound.refresh(rejecting: "b-token") == nil)
        // A refresh would have marked the credentials pending before contacting the token endpoint.
        #expect(store.load() == saved)
    }

    @Test func aSignInDuringARefreshForAnotherServerIsNotRefreshed() async throws {
        CountingTokenURLProtocol.reset()
        let replacement = credentials(for: signedIn, prefix: "b", expiresIn: -1)
        let store = SwitchingStore(
            first: credentials(for: remembered, prefix: "a", expiresIn: -1), next: replacement)
        let transport = transport()
        defer { transport.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: transport, now: { clock })

        #expect(try await oauth.refresh(rejecting: "a-token", for: remembered) == nil)
        #expect(CountingTokenURLProtocol.requests == 0)
        #expect(store.load() == replacement)
    }

    @Test func aStalePendingSaveDoesNotHideANewerServer() async throws {
        CountingTokenURLProtocol.reset()
        let store = FailingRotationStore(credentials(for: remembered, prefix: "a", expiresIn: -1))
        let transport = transport()
        defer { transport.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: transport, now: { clock })
        await #expect(throws: CocoaError.self) { try await oauth.token() }
        try store.save(credentials(for: signedIn, prefix: "b", expiresIn: 3600))

        #expect(try await oauth.token(for: signedIn) == "b-token")
        #expect(try await oauth.token(for: remembered) == nil)
        #expect(CountingTokenURLProtocol.requests == 1)
    }
}
