import Foundation
import Testing

@testable import FolioKit

private final class TokenEndpointURLProtocol: URLProtocol, @unchecked Sendable {
    enum Outcome: Sendable {
        case reply(status: Int, body: String)
        case failure(URLError.Code)
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var outcome = Outcome.reply(status: 200, body: "")
    nonisolated(unsafe) private static var count = 0
    nonisolated(unsafe) private static var beforeReply: (@Sendable () async -> Void)?

    static func reset(_ outcome: Outcome, beforeReply: (@Sendable () async -> Void)? = nil) {
        lock.withLock {
            Self.outcome = outcome
            Self.beforeReply = beforeReply
            count = 0
        }
    }

    static var requests: Int { lock.withLock { count } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (outcome, beforeReply) = Self.lock.withLock {
            Self.count += 1
            return (Self.outcome, Self.beforeReply)
        }
        if let beforeReply {
            Task { @Sendable [self] in
                await beforeReply()
                complete(outcome)
            }
        } else {
            complete(outcome)
        }
    }

    private func complete(_ outcome: Outcome) {
        switch outcome {
        case let .failure(code):
            client?.urlProtocol(self, didFailWithError: URLError(code))
        case let .reply(status, body):
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
    }
    override func stopLoading() {}
}

private struct PendingReadStore: CredentialStore {
    let base: InMemoryCredentialStore
    let pendingRead: AsyncStream<Void>.Continuation

    func load() -> OAuthCredentials? {
        let credentials = base.load()
        if credentials?.refreshPending == true { pendingRead.yield(()) }
        return credentials
    }

    func save(_ credentials: OAuthCredentials) { base.save(credentials) }
    func clear() { base.clear() }
}

private final class RestoreFailingStore: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var credentials: OAuthCredentials?
    private var saves = 0

    init(_ credentials: OAuthCredentials) { self.credentials = credentials }
    func load() -> OAuthCredentials? { lock.withLock { credentials } }
    func save(_ credentials: OAuthCredentials) throws {
        try lock.withLock {
            saves += 1
            // The first save marks the refresh pending; the second tries to restore after the failure.
            if saves == 2 { throw CocoaError(.fileWriteNoPermission) }
            self.credentials = credentials
        }
    }
    func clear() { lock.withLock { credentials = nil } }
}

@Suite(.serialized, .timeLimit(.minutes(1))) struct OAuthTransientRefreshTests {
    private let clock = Date(timeIntervalSince1970: 1_800_000_000)
    private let success = #"{"access_token":"new","refresh_token":"r2","expires_in":3600}"#

    private func credentials() -> OAuthCredentials {
        OAuthCredentials(
            serverURL: URL(string: "https://folio.example.com")!, clientID: "client",
            tokenEndpoint: URL(string: "https://folio.example.com/oauth/token")!, revocationEndpoint: nil,
            tokens: OAuthTokens(accessToken: "old", refreshToken: "r1", expiresAt: clock.addingTimeInterval(-1)))
    }

    private func oauth(_ store: any CredentialStore) -> OAuthSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [TokenEndpointURLProtocol.self]
        return OAuthSession(store: store, session: URLSession(configuration: configuration), now: { clock })
    }

    /// The credentials are untouched and the next call refreshes again.
    private func expectRecovers(_ oauth: OAuthSession, _ store: InMemoryCredentialStore) async throws {
        #expect(store.load()?.tokens.refreshToken == "r1")
        #expect(store.load()?.refreshPending != true)
        TokenEndpointURLProtocol.reset(.reply(status: 200, body: success))
        #expect(try await oauth.token() == "new")
        #expect(store.load()?.tokens.refreshToken == "r2")
    }

    @Test(arguments: [500, 502, 503, 504])
    func aServerErrorKeepsTheUserSignedIn(status: Int) async throws {
        TokenEndpointURLProtocol.reset(.reply(status: status, body: "<html>down</html>"))
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(store)
        await #expect(throws: FolioError.server(status: status, message: nil)) { _ = try await oauth.token() }
        try await expectRecovers(oauth, store)
    }

    @Test(arguments: [
        URLError.Code.timedOut, .networkConnectionLost, .notConnectedToInternet, .secureConnectionFailed, .cancelled,
    ])
    func aTimeoutOrDroppedConnectionKeepsTheUserSignedIn(code: URLError.Code) async throws {
        TokenEndpointURLProtocol.reset(.failure(code))
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(store)
        await #expect(throws: FolioError.unreachable) { _ = try await oauth.token() }
        try await expectRecovers(oauth, store)
    }

    @Test func rateLimitingKeepsTheUserSignedIn() async throws {
        TokenEndpointURLProtocol.reset(.reply(status: 429, body: #"{"error":"slow_down"}"#))
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(store)
        await #expect(throws: FolioError.server(status: 429, message: nil)) { _ = try await oauth.token() }
        try await expectRecovers(oauth, store)
    }

    @Test(arguments: [400, 401])
    func aRevokedRefreshTokenStillSignsTheUserOut(status: Int) async throws {
        TokenEndpointURLProtocol.reset(.reply(status: status, body: #"{"error":"invalid_grant"}"#))
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(store)
        #expect(try await oauth.token() == nil)
        #expect(store.load()?.tokens.refreshToken == nil)
        #expect(store.load()?.isSignedIn == false)
    }

    @Test(arguments: [(401, "invalid_client"), (400, "unauthorized_client"), (400, "unsupported_grant_type")])
    func anyDefinitiveOAuthRefusalSignsTheUserOut(status: Int, error: String) async throws {
        TokenEndpointURLProtocol.reset(.reply(status: status, body: "{\"error\":\"\(error)\"}"))
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(store)
        #expect(try await oauth.token() == nil)
        #expect(store.load()?.isSignedIn == false)
        #expect(store.load()?.refreshPending != true)
    }

    @Test(arguments: [
        (404, "<html>not found</html>"), (403, ""), (405, "nope"), (410, ""), (200, "<html>portal</html>"),
    ])
    func aProxyAnswerOnTheTokenEndpointNeverLooksLikeAnAPIStatus(status: Int, body: String) async throws {
        TokenEndpointURLProtocol.reset(.reply(status: status, body: body))
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(store)
        await #expect(throws: FolioError.unreadableResponse) { _ = try await oauth.token() }
        try await expectRecovers(oauth, store)
    }

    @Test func aFailedRestoreWriteStillRefreshesOnTheNextCall() async throws {
        TokenEndpointURLProtocol.reset(.reply(status: 503, body: ""))
        let store = RestoreFailingStore(credentials())
        let oauth = oauth(store)
        await #expect(throws: CocoaError.self) { _ = try await oauth.token() }
        TokenEndpointURLProtocol.reset(.reply(status: 200, body: success))
        #expect(try await oauth.token() == "new")
        #expect(TokenEndpointURLProtocol.requests == 1)
    }

    @Test func concurrentCallersShareOneTransientOutcome() async throws {
        let (transferStarted, signalStarted) = AsyncStream<Void>.makeStream()
        let (pendingReads, signalRead) = AsyncStream<Void>.makeStream()
        defer {
            signalStarted.finish()
            signalRead.finish()
        }
        TokenEndpointURLProtocol.reset(.reply(status: 503, body: "")) {
            signalStarted.yield(())
            for await _ in pendingReads.prefix(3) {}
        }
        let store = InMemoryCredentialStore(credentials())
        let oauth = oauth(PendingReadStore(base: store, pendingRead: signalRead))
        let readToken: @Sendable () async -> Bool = {
            do { _ = try await oauth.token(); return false } catch {
                return error as? FolioError == .server(status: 503, message: nil)
            }
        }
        let outcomes = await withTaskGroup(of: Bool.self) { group in
            group.addTask(operation: readToken)
            for await _ in transferStarted.prefix(1) {}
            for _ in 0..<3 { group.addTask(operation: readToken) }
            return await group.reduce(into: []) { $0.append($1) }
        }
        #expect(outcomes.allSatisfy { $0 })
        #expect(TokenEndpointURLProtocol.requests == 1)
        #expect(store.load()?.tokens.refreshToken == "r1")
        #expect(store.load()?.refreshPending != true)
    }
}
