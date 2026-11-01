import Foundation

/// The signed-in user's bearer token: hands out the access token, refreshes it when it is about to expire
/// or the server rejects it, and rotates the refresh token in the shared store.
public actor OAuthSession: BearerTokenProvider {
    private let store: any CredentialStore
    private let client: OAuthClient
    private let now: @Sendable () -> Date
    private var inFlight: Task<String?, Never>?

    /// Refresh this long before the access token expires so a request never races the deadline.
    private let leeway: TimeInterval = 30

    public init(
        store: any CredentialStore, session: URLSession = .shared, now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.store = store
        self.now = now
        client = OAuthClient(session: session, now: now)
    }

    public func token() async -> String? {
        guard let credentials = store.load(), credentials.isSignedIn else { return nil }
        if credentials.tokens.expiresAt.timeIntervalSince(now()) > leeway {
            return credentials.tokens.accessToken
        }
        return await refresh(rejecting: credentials.tokens.accessToken)
    }

    public func refresh(rejecting token: String?) async -> String? {
        if let inFlight {
            return await inFlight.value
        }
        guard let credentials = store.load(), credentials.isSignedIn else { return nil }
        // Another caller, or another process such as an extension, already rotated past the rejected token.
        if let token, credentials.tokens.accessToken != token,
            credentials.tokens.expiresAt.timeIntervalSince(now()) > leeway
        {
            return credentials.tokens.accessToken
        }
        let task = Task { await performRefresh(credentials) }
        inFlight = task
        let result = await task.value
        inFlight = nil
        return result
    }

    /// Clears the tokens and asks the server to revoke the refresh token; the revocation is best effort.
    public func signOut() async {
        guard let credentials = store.load() else { return }
        store.clear()
        if let endpoint = credentials.revocationEndpoint, let refresh = credentials.tokens.refreshToken {
            await client.revoke(
                OAuthRequests.revocation(endpoint: endpoint, clientID: credentials.clientID, token: refresh)
            )
        }
    }

    private func performRefresh(_ credentials: OAuthCredentials) async -> String? {
        guard let refreshToken = credentials.tokens.refreshToken else { return nil }
        let request = OAuthRequests.refresh(
            endpoint: credentials.tokenEndpoint, clientID: credentials.clientID, refreshToken: refreshToken
        )
        do {
            var updated = credentials
            updated.tokens = try await client.tokens(for: request, keeping: refreshToken)
            store.save(updated)
            return updated.tokens.accessToken
        } catch OAuthFailure.invalidGrant {
            // Rotation means a concurrent refresh elsewhere invalidates this token; prefer what that left behind.
            if let latest = store.load(), latest.tokens.refreshToken != refreshToken, latest.isSignedIn {
                return latest.tokens.accessToken
            }
            var signedOut = credentials
            signedOut.tokens = .signedOut
            store.save(signedOut)
            return nil
        } catch {
            return nil
        }
    }
}
