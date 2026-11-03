import Foundation
import Testing

@testable import FolioKit

struct ServerBoundTokenTests {
    private let signedIn = URL(string: "https://b.example.com")!
    private let remembered = URL(string: "https://a.example.com")!
    private let clock = Date(timeIntervalSince1970: 1_800_000_000)

    private func store(expiresIn: TimeInterval = 3600) -> InMemoryCredentialStore {
        InMemoryCredentialStore(
            OAuthCredentials(
                serverURL: signedIn, clientID: "client", tokenEndpoint: signedIn.appending(path: "oauth/token"),
                revocationEndpoint: nil,
                tokens: OAuthTokens(
                    accessToken: "b-token", refreshToken: "b-refresh", expiresAt: clock.addingTimeInterval(expiresIn)
                )
            )
        )
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
}
