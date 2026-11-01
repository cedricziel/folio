import Foundation

/// Opens the authorization page and returns the URL the server redirected to.
/// The app implements this with `ASWebAuthenticationSession`.
public protocol WebAuthenticating: Sendable {
    func authenticate(url: URL, callbackScheme: String) async throws -> URL
}

/// The interactive half of OAuth 2.1: discovery, dynamic client registration, authorization code with PKCE.
public struct OAuthSignIn: Sendable {
    private let store: any CredentialStore
    private let clients: any ClientRegistry
    private let authenticator: any WebAuthenticating
    private let client: OAuthClient

    public init(
        store: any CredentialStore, clients: any ClientRegistry, authenticator: any WebAuthenticating,
        session: URLSession = .shared
    ) {
        self.store = store
        self.clients = clients
        self.authenticator = authenticator
        client = OAuthClient(session: session, now: { Date() })
    }

    public func signIn(to server: URL) async throws {
        let metadata = try await client.discover(server)
        let resource = await client.resource(for: server)

        let clientID: String
        if let cached = clients.clientID(for: server) {
            clientID = cached
        } else {
            clientID = try await client.register(metadata)
            clients.save(clientID: clientID, for: server)
        }

        let pkce = PKCE.generate()
        let state = PKCE.generate().verifier
        let authorizeURL = OAuthRequests.authorizationURL(
            metadata: metadata, clientID: clientID, redirectURI: OAuthConfiguration.redirectURI,
            resource: resource, challenge: pkce.challenge, state: state
        )
        let callback = try await authenticator.authenticate(
            url: authorizeURL, callbackScheme: OAuthConfiguration.callbackScheme
        )

        let items = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func value(_ name: String) -> String? {
            items.first { $0.name == name }?.value
        }
        guard value("state") == state else {
            throw FolioError.signInFailed("The sign-in answer didn't match the request.")
        }
        guard value("error") == nil else { throw FolioError.signInFailed("Sign-in was cancelled or denied.") }
        guard let code = value("code") else {
            throw FolioError.signInFailed("The server didn't return a sign-in code.")
        }

        let tokens: OAuthTokens
        do {
            tokens = try await client.tokens(
                for: OAuthRequests.tokenExchange(
                    endpoint: metadata.tokenEndpoint, clientID: clientID, code: code, verifier: pkce.verifier,
                    redirectURI: OAuthConfiguration.redirectURI, resource: resource
                )
            )
        } catch OAuthFailure.invalidGrant {
            throw FolioError.signInFailed("The sign-in code was refused. Try again.")
        } catch let failure as OAuthFailure {
            throw FolioError.signInFailed("The server refused the sign-in: \(failure).")
        }
        store.save(
            OAuthCredentials(
                serverURL: server, clientID: clientID, tokenEndpoint: metadata.tokenEndpoint,
                revocationEndpoint: metadata.revocationEndpoint, tokens: tokens
            )
        )
    }
}
