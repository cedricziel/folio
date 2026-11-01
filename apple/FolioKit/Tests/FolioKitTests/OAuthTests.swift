import Foundation
import Testing

@testable import FolioKit

struct PKCETests {
    @Test func challengeMatchesTheRFC7636Example() {
        #expect(
            PKCE.challenge(for: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
                == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        )
    }

    @Test func generatedVerifiersAreUniqueUnreservedAndLongEnough() {
        let first = PKCE.generate()
        let second = PKCE.generate()
        #expect(first.verifier != second.verifier)
        #expect((43...128).contains(first.verifier.count))
        let allowed = CharacterSet(
            charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
        )
        #expect(first.verifier.unicodeScalars.allSatisfy(allowed.contains))
        #expect(first.challenge == PKCE.challenge(for: first.verifier))
    }
}

struct CredentialStoreTests {
    static let credentials = OAuthCredentials(
        serverURL: URL(string: "https://folio.example.com")!, clientID: "client-1",
        tokenEndpoint: URL(string: "https://folio.example.com/oauth/token")!, revocationEndpoint: nil,
        tokens: OAuthTokens(accessToken: "a", refreshToken: "r", expiresAt: Date(timeIntervalSince1970: 2_000_000_000))
    )

    @Test func inMemoryStoreRoundTrips() {
        let store = InMemoryCredentialStore()
        #expect(store.load() == nil)
        store.save(Self.credentials)
        #expect(store.load() == Self.credentials)
        store.clear()
        #expect(store.load() == nil)
    }

    @Test func keychainStoreRoundTripsAndOverwritesAtomically() {
        let store = KeychainCredentialStore(
            service: "com.cedricziel.folio.tests.\(UUID().uuidString)", accessGroup: nil, dataProtection: false
        )
        defer { store.clear() }
        #expect(store.load() == nil)
        store.save(Self.credentials)
        #expect(store.load() == Self.credentials)

        var rotated = Self.credentials
        rotated.tokens = OAuthTokens(
            accessToken: "b", refreshToken: "s", expiresAt: Date(timeIntervalSince1970: 2_000_000_100))
        store.save(rotated)
        #expect(store.load() == rotated)
        store.clear()
        #expect(store.load() == nil)
    }
}

struct OAuthRequestTests {
    private let metadata = OAuthMetadata(
        issuer: "https://folio.example.com",
        authorizationEndpoint: URL(string: "https://folio.example.com/oauth/authorize")!,
        tokenEndpoint: URL(string: "https://folio.example.com/oauth/token")!,
        registrationEndpoint: URL(string: "https://folio.example.com/oauth/register")!,
        revocationEndpoint: URL(string: "https://folio.example.com/oauth/revoke")
    )

    @Test func registrationRequestIsAPublicClientWithACustomSchemeRedirect() throws {
        let request = try OAuthRequests.registration(
            endpoint: metadata.registrationEndpoint, redirectURI: OAuthConfiguration.redirectURI, clientName: "Folio"
        )
        #expect(request.httpMethod == "POST")
        #expect(request.url == metadata.registrationEndpoint)
        #expect(request.value(forHTTPHeaderField: "Content-Type") == "application/json")
        let data = try #require(request.httpBody)
        let body = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(body["client_name"] as? String == "Folio")
        #expect(body["redirect_uris"] as? [String] == ["com.cedricziel.folio:/oauth/callback"])
        #expect(body["token_endpoint_auth_method"] as? String == "none")
        #expect(body["grant_types"] as? [String] == ["authorization_code", "refresh_token"])
        #expect(body["response_types"] as? [String] == ["code"])
    }

    @Test func authorizationURLCarriesPKCEResourceAndScopes() throws {
        let url = OAuthRequests.authorizationURL(
            metadata: metadata, clientID: "client-1", redirectURI: OAuthConfiguration.redirectURI,
            resource: "https://folio.example.com/api", challenge: "chal", state: "st"
        )
        let queryItems = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems)
        let items = Dictionary(uniqueKeysWithValues: queryItems.map { ($0.name, $0.value ?? "") })
        #expect(url.path == "/oauth/authorize")
        #expect(items["response_type"] == "code")
        #expect(items["client_id"] == "client-1")
        #expect(items["redirect_uri"] == "com.cedricziel.folio:/oauth/callback")
        #expect(items["code_challenge"] == "chal")
        #expect(items["code_challenge_method"] == "S256")
        #expect(items["scope"] == "documents:read documents:write")
        #expect(items["resource"] == "https://folio.example.com/api")
        #expect(items["state"] == "st")
    }

    @Test func tokenRequestsAreFormEncoded() throws {
        let exchange = OAuthRequests.tokenExchange(
            endpoint: metadata.tokenEndpoint, clientID: "c", code: "the code", verifier: "ver",
            redirectURI: OAuthConfiguration.redirectURI, resource: "https://folio.example.com/api"
        )
        #expect(exchange.value(forHTTPHeaderField: "Content-Type") == "application/x-www-form-urlencoded")
        let text = try String(decoding: #require(exchange.httpBody), as: UTF8.self)
        #expect(text.contains("grant_type=authorization_code"))
        #expect(text.contains("code=the%20code"))
        #expect(text.contains("code_verifier=ver"))

        let refresh = OAuthRequests.refresh(endpoint: metadata.tokenEndpoint, clientID: "c", refreshToken: "r1")
        let refreshText = try String(decoding: #require(refresh.httpBody), as: UTF8.self)
        #expect(refreshText.contains("grant_type=refresh_token"))
        #expect(refreshText.contains("refresh_token=r1"))
    }
}

/// Routes by path so one handler can play the whole authorization server and the API.
final class RouterURLProtocol: URLProtocol, @unchecked Sendable {
    struct Reply {
        var status = 200
        var body = Data()
    }

    nonisolated(unsafe) static var handler: ((URLRequest, Data) -> Reply)?

    override class func canInit(with _: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        var body = request.httpBody ?? Data()
        if let stream = request.httpBodyStream {
            stream.open()
            var buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable {
                let count = stream.read(&buffer, maxLength: buffer.count)
                if count <= 0 {
                    break
                }
                body.append(buffer, count: count)
            }
            stream.close()
        }
        let reply = Self.handler?(request, body) ?? Reply(status: 500)
        let response = HTTPURLResponse(
            url: request.url!, statusCode: reply.status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: reply.body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

@Suite(.serialized) struct OAuthNetworkTests {
    @Suite(.serialized) struct Session {
        private let server = URL(string: "https://folio.example.com")!
        private let clock = Date(timeIntervalSince1970: 1_800_000_000)

        private func session() -> URLSession {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.protocolClasses = [RouterURLProtocol.self]
            return URLSession(configuration: configuration)
        }

        private func credentials(access: String = "old", refresh: String = "r1", expiresIn: TimeInterval = 3600)
            -> OAuthCredentials
        {
            OAuthCredentials(
                serverURL: server, clientID: "client-1", tokenEndpoint: server.appending(path: "oauth/token"),
                revocationEndpoint: server.appending(path: "oauth/revoke"),
                tokens: OAuthTokens(
                    accessToken: access, refreshToken: refresh, expiresAt: clock.addingTimeInterval(expiresIn))
            )
        }

        private func tokenReply(access: String, refresh: String) -> RouterURLProtocol.Reply {
            .init(
                body: Data(
                    "{\"access_token\":\"\(access)\",\"refresh_token\":\"\(refresh)\",\"token_type\":\"Bearer\",\"expires_in\":3600}"
                        .utf8
                )
            )
        }

        @Test func a401RefreshesRotatesAndRetriesOnce() async throws {
            let store = InMemoryCredentialStore(credentials())
            var seenTokens: [String] = []
            var refreshBodies: [String] = []
            RouterURLProtocol.handler = { request, body in
                if request.url?.path == "/oauth/token" {
                    refreshBodies.append(String(decoding: body, as: UTF8.self))
                    return self.tokenReply(access: "new", refresh: "r2")
                }
                let token = request.value(forHTTPHeaderField: "Authorization") ?? ""
                seenTokens.append(token)
                return token == "Bearer new"
                    ? .init(body: Data("[]".utf8)) : .init(status: 401, body: Data("{\"error\":\"nope\"}".utf8))
            }
            let oauth = OAuthSession(store: store, session: session(), now: { clock })
            let service = LiveFolioService(baseURL: server, tokenProvider: oauth, session: session())

            _ = try await service.tags()

            #expect(seenTokens == ["Bearer old", "Bearer new"])
            #expect(refreshBodies.count == 1)
            #expect(refreshBodies[0].contains("refresh_token=r1"))
            #expect(store.load()?.tokens.accessToken == "new")
            #expect(store.load()?.tokens.refreshToken == "r2")
        }

        @Test func rawRequestsRetryAfterARefreshToo() async throws {
            let store = InMemoryCredentialStore(credentials())
            RouterURLProtocol.handler = { request, _ in
                if request.url?.path == "/oauth/token" {
                    return self.tokenReply(access: "new", refresh: "r2")
                }
                return request.value(forHTTPHeaderField: "Authorization") == "Bearer new"
                    ? .init(body: Data("%PDF".utf8)) : .init(status: 401)
            }
            let oauth = OAuthSession(store: store, session: session(), now: { clock })
            let service = LiveFolioService(baseURL: server, tokenProvider: oauth, session: session())
            let url = try await service.downloadFile(for: Document(title: "x", originalFilename: "x.pdf"))
            #expect(try Data(contentsOf: url) == Data("%PDF".utf8))
        }

        @Test func anExpiredAccessTokenIsRefreshedBeforeTheRequest() async throws {
            let store = InMemoryCredentialStore(credentials(expiresIn: -10))
            var apiTokens: [String] = []
            RouterURLProtocol.handler = { request, _ in
                if request.url?.path == "/oauth/token" {
                    return self.tokenReply(access: "fresh", refresh: "r2")
                }
                apiTokens.append(request.value(forHTTPHeaderField: "Authorization") ?? "")
                return .init(body: Data("[]".utf8))
            }
            let oauth = OAuthSession(store: store, session: session(), now: { clock })
            _ = try await LiveFolioService(baseURL: server, tokenProvider: oauth, session: session()).tags()
            #expect(apiTokens == ["Bearer fresh"])
        }

        @Test func parallelRejectionsShareOneRefresh() async throws {
            let store = InMemoryCredentialStore(credentials())
            let counter = Counter()
            RouterURLProtocol.handler = { request, _ in
                if request.url?.path == "/oauth/token" {
                    counter.increment()
                    return self.tokenReply(access: "new", refresh: "r2")
                }
                return request.value(forHTTPHeaderField: "Authorization") == "Bearer new"
                    ? .init(body: Data("[]".utf8)) : .init(status: 401)
            }
            let oauth = OAuthSession(store: store, session: session(), now: { clock })
            let service = LiveFolioService(baseURL: server, tokenProvider: oauth, session: session())
            async let a = service.tags()
            async let b = service.correspondents()
            _ = try await (a, b)
            #expect(counter.value == 1)
        }

        @Test func aRevokedRefreshTokenSignsTheUserOutAndAsksForSignIn() async throws {
            let store = InMemoryCredentialStore(credentials())
            RouterURLProtocol.handler = { request, _ in
                if request.url?.path == "/oauth/token" {
                    return .init(status: 400, body: Data("{\"error\":\"invalid_grant\"}".utf8))
                }
                return .init(status: 401)
            }
            let oauth = OAuthSession(store: store, session: session(), now: { clock })
            let service = LiveFolioService(baseURL: server, tokenProvider: oauth, session: session())
            await #expect(throws: FolioError.signInRequired) { _ = try await service.tags() }
            #expect(store.load()?.tokens.refreshToken == nil)
        }

        @Test func signOutClearsCredentialsAndRevokesBestEffort() async {
            let store = InMemoryCredentialStore(credentials())
            var revoked = false
            RouterURLProtocol.handler = { request, body in
                if request.url?.path == "/oauth/revoke" {
                    revoked = String(decoding: body, as: UTF8.self).contains("token=r1")
                }
                return .init()
            }
            let oauth = OAuthSession(store: store, session: session(), now: { clock })
            await oauth.signOut()
            #expect(store.load() == nil)
            #expect(revoked)
        }
    }

    final class Counter: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        var value: Int {
            lock.withLock { count }
        }

        func increment() {
            lock.withLock { count += 1 }
        }
    }

    final class FakeAuthenticator: WebAuthenticating, @unchecked Sendable {
        var visited: [URL] = []
        var stateOverride: String?
        var errorCallback = false

        func authenticate(url: URL, callbackScheme: String) async throws -> URL {
            visited.append(url)
            let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
            let state = stateOverride ?? items.first { $0.name == "state" }?.value ?? ""
            let query = errorCallback ? "error=access_denied&state=\(state)" : "code=abc&state=\(state)"
            return URL(string: "\(callbackScheme):/oauth/callback?\(query)")!
        }
    }

    @Suite(.serialized) struct SignIn {
        private let server = URL(string: "https://folio.example.com")!

        private func session() -> URLSession {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.protocolClasses = [RouterURLProtocol.self]
            return URLSession(configuration: configuration)
        }

        private func installServer(registrations: Counter, exchanged: @escaping (String) -> Void = { _ in }) {
            RouterURLProtocol.handler = { request, body in
                switch request.url?.path {
                case "/.well-known/oauth-authorization-server":
                    return .init(
                        body: Data(
                            """
                            {"issuer":"https://folio.example.com",
                             "authorization_endpoint":"https://folio.example.com/oauth/authorize",
                             "token_endpoint":"https://folio.example.com/oauth/token",
                             "registration_endpoint":"https://folio.example.com/oauth/register",
                             "revocation_endpoint":"https://folio.example.com/oauth/revoke"}
                            """.utf8
                        )
                    )
                case "/.well-known/oauth-protected-resource/api":
                    return .init(
                        body: Data("{\"resource\":\"https://folio.example.com/api\"}".utf8)
                    )
                case "/oauth/register":
                    registrations.increment()
                    return .init(status: 201, body: Data("{\"client_id\":\"client-9\"}".utf8))
                case "/oauth/token":
                    exchanged(String(decoding: body, as: UTF8.self))
                    return .init(
                        body: Data(
                            "{\"access_token\":\"acc\",\"refresh_token\":\"ref\",\"token_type\":\"Bearer\",\"expires_in\":3600}"
                                .utf8
                        )
                    )
                default:
                    return .init(status: 404)
                }
            }
        }

        @Test func signInDiscoversRegistersAuthorizesAndStoresTheTokens() async throws {
            let registrations = Counter()
            var exchange = ""
            installServer(registrations: registrations) { exchange = $0 }
            let store = InMemoryCredentialStore()
            let authenticator = FakeAuthenticator()
            let signIn = OAuthSignIn(
                store: store, clients: InMemoryClientRegistry(), authenticator: authenticator, session: session()
            )

            try await signIn.signIn(to: server)

            let visited = try #require(authenticator.visited.first)
            let query = URLComponents(url: visited, resolvingAgainstBaseURL: false)?.queryItems ?? []
            #expect(query.first { $0.name == "client_id" }?.value == "client-9")
            #expect(query.first { $0.name == "resource" }?.value == "https://folio.example.com/api")
            #expect(exchange.contains("code=abc"))
            #expect(exchange.contains("code_verifier="))
            #expect(store.load()?.tokens.accessToken == "acc")
            #expect(store.load()?.tokens.refreshToken == "ref")
            #expect(store.load()?.clientID == "client-9")
        }

        @Test func theClientRegistrationIsCachedPerServer() async throws {
            let registrations = Counter()
            installServer(registrations: registrations)
            let clients = InMemoryClientRegistry()
            for _ in 0..<2 {
                let signIn = OAuthSignIn(
                    store: InMemoryCredentialStore(), clients: clients, authenticator: FakeAuthenticator(),
                    session: session()
                )
                try await signIn.signIn(to: server)
            }
            #expect(registrations.value == 1)
        }

        @Test func aMismatchedStateIsRejected() async {
            installServer(registrations: Counter())
            let authenticator = FakeAuthenticator()
            authenticator.stateOverride = "forged"
            let store = InMemoryCredentialStore()
            let signIn = OAuthSignIn(
                store: store, clients: InMemoryClientRegistry(), authenticator: authenticator, session: session()
            )
            await #expect(throws: FolioError.signInFailed("The sign-in answer didn't match the request.")) {
                try await signIn.signIn(to: server)
            }
            #expect(store.load() == nil)
        }

        @Test func aDeniedAuthorizationIsReported() async {
            installServer(registrations: Counter())
            let authenticator = FakeAuthenticator()
            authenticator.errorCallback = true
            let signIn = OAuthSignIn(
                store: InMemoryCredentialStore(), clients: InMemoryClientRegistry(), authenticator: authenticator,
                session: session()
            )
            await #expect(throws: FolioError.signInFailed("Sign-in was cancelled or denied.")) {
                try await signIn.signIn(to: server)
            }
        }
    }
}
