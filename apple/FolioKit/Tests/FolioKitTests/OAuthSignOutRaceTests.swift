import Foundation
import Testing

@testable import FolioKit

/// Holds every token-endpoint request until the test releases it; other requests (revocation) answer at once.
///
/// A held request waits without occupying a thread, so held requests never starve the thread pools other tests
/// run on.
private final class HeldTokenURLProtocol: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    private nonisolated(unsafe) static var status = 200
    private nonisolated(unsafe) static var body = ""
    private nonisolated(unsafe) static var waiting: [HeldTokenURLProtocol] = []
    /// Releases that arrived before a request to answer, like signals on a semaphore.
    private nonisolated(unsafe) static var releases = 0
    private nonisolated(unsafe) static var held = 0

    static func reset(status: Int, body: String) {
        lock.withLock {
            Self.status = status
            Self.body = body
            waiting = []
            releases = 0
            held = 0
        }
    }

    static var heldRequests: Int {
        lock.withLock { held }
    }

    /// Answers the oldest held token request, or the next one to arrive.
    static func release() {
        let next = lock.withLock { () -> (HeldTokenURLProtocol, Int, String)? in
            guard !waiting.isEmpty else {
                releases += 1
                return nil
            }
            return (waiting.removeFirst(), status, body)
        }
        if case let (request, status, body)? = next {
            request.respond(status: status, body: body)
        }
    }

    override class func canInit(with _: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard request.url?.path == "/oauth/token" else { return respond(status: 200, body: "") }
        let answer = Self.lock.withLock { () -> (Int, String)? in
            Self.held += 1
            guard Self.releases > 0 else {
                Self.waiting.append(self)
                return nil
            }
            Self.releases -= 1
            return (Self.status, Self.body)
        }
        if case let (status, body)? = answer {
            respond(status: status, body: body)
        }
    }

    private func respond(status: Int, body: String) {
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

@Suite(.serialized) struct OAuthSignOutRaceTests {
    private let clock = Date(timeIntervalSince1970: 1_800_000_000)
    private let rotated = #"{"access_token":"new","refresh_token":"r2","token_type":"Bearer","expires_in":3600}"#
    private let refused = #"{"error":"invalid_grant"}"#

    private func credentials(access: String = "old", refresh: String = "r1") -> OAuthCredentials {
        let server = URL(string: "https://folio.example.com")!
        return OAuthCredentials(
            serverURL: server, clientID: "client", tokenEndpoint: server.appending(path: "oauth/token"),
            revocationEndpoint: server.appending(path: "oauth/revoke"),
            tokens: OAuthTokens(accessToken: access, refreshToken: refresh, expiresAt: clock.addingTimeInterval(-1))
        )
    }

    private func transport() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [HeldTokenURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    private func lock() throws -> (OAuthRefreshLock, URL) {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return (OAuthRefreshLock(url: directory.appending(path: "refresh.lock")), directory)
    }

    private func until(_ condition: () async -> Bool) async throws {
        for _ in 0..<2000 where !(await condition()) {
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(await condition())
    }

    @Test(arguments: [true, false])
    func signOutDuringARotatingRefreshStaysSignedOut(rotates: Bool) async throws {
        HeldTokenURLProtocol.reset(status: rotates ? 200 : 400, body: rotates ? rotated : refused)
        let (refreshLock, directory) = try lock()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = InMemoryCredentialStore(credentials())
        let session = transport()
        defer { session.invalidateAndCancel() }
        let oauth = OAuthSession(store: store, session: session, now: { clock }, refreshLock: refreshLock)

        let request = Task { try await oauth.token() }
        try await until { HeldTokenURLProtocol.heldRequests == 1 }
        let signOut = Task { try await oauth.signOut() }
        try await until { await oauth.isSigningOut }
        HeldTokenURLProtocol.release()

        #expect(try await request.value == nil)
        try await signOut.value
        #expect(store.load() == nil)
        #expect(try await oauth.token() == nil)
        #expect(store.load() == nil)
    }

    @Test func signOutInAnotherProcessWaitsForItsRefreshAndWins() async throws {
        HeldTokenURLProtocol.reset(status: 200, body: rotated)
        let (refreshLock, directory) = try lock()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = InMemoryCredentialStore(credentials())
        let session = transport()
        defer { session.invalidateAndCancel() }
        let fileExtension = OAuthSession(store: store, session: session, now: { clock }, refreshLock: refreshLock)
        let app = OAuthSession(store: store, session: session, now: { clock }, refreshLock: refreshLock)

        let request = Task { try await fileExtension.token() }
        try await until { HeldTokenURLProtocol.heldRequests == 1 }
        let signOut = Task { try await app.signOut() }
        HeldTokenURLProtocol.release()

        _ = try await request.value
        try await signOut.value
        #expect(store.load() == nil)
        #expect(try await fileExtension.token() == nil)
        #expect(store.load() == nil)
    }

    @Test(arguments: [true, false])
    func aRefreshCannotOverwriteASignInThatSupersededIt(locked: Bool) async throws {
        HeldTokenURLProtocol.reset(status: 200, body: rotated)
        let (refreshLock, directory) = try lock()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = InMemoryCredentialStore(credentials())
        let session = transport()
        defer { session.invalidateAndCancel() }
        let oauth = OAuthSession(
            store: store, session: session, now: { clock }, refreshLock: locked ? refreshLock : nil
        )
        var newer = credentials(access: "another-account", refresh: "another-refresh")
        newer.tokens.expiresAt = clock.addingTimeInterval(3600)

        let request = Task { try await oauth.token() }
        try await until { HeldTokenURLProtocol.heldRequests == 1 }
        // The final write of OAuthSignIn.signIn(to:).
        // Locked, the sign-in waits for the refresh; unlocked, it lands while the refresh is in flight.
        let signIn = locked ? Task { [newer] in try await refreshLock.withLock { store.save(newer) } } : nil
        if !locked { store.save(newer) }
        HeldTokenURLProtocol.release()

        _ = try await request.value
        try await signIn?.value
        #expect(store.load() == newer)
        #expect(try await oauth.token() == "another-account")
    }
}
