import Foundation

/// Refreshes credentials and reports storage and transport failures without treating them as sign-out.
public actor OAuthSession: BearerTokenProvider {
    private let store: any CredentialStore
    private let client: OAuthClient
    private let now: @Sendable () -> Date
    private let refreshLock: OAuthRefreshLock?
    private var inFlight: Task<String?, Error>?
    private var pendingSave: OAuthCredentials?
    private var pendingBaseline: OAuthCredentials?
    private var signingOut = false
    private var signOuts = 0
    private let leeway: TimeInterval = 30

    public init(
        store: any CredentialStore, session: URLSession = .shared,
        now: @escaping @Sendable () -> Date = { Date() }, refreshLock: OAuthRefreshLock? = nil
    ) {
        self.store = store
        self.now = now
        self.refreshLock = refreshLock
        client = OAuthClient(session: session, now: now)
    }

    public func token() async throws -> String? {
        guard !signingOut else { return nil }
        if pendingSave != nil { return try await refresh(rejecting: nil) }
        let credentials: OAuthCredentials?
        do { credentials = try store.load() } catch {
            changed()
            throw error
        }
        guard let credentials, credentials.isSignedIn else {
            changed()
            return nil
        }
        if let token = usableToken(credentials) { return token }
        return try await refresh(rejecting: credentials.tokens.accessToken)
    }

    public func refresh(rejecting token: String?) async throws -> String? {
        guard !signingOut else { return nil }
        let generation = signOuts
        let refreshed = try await sharedRefresh(rejecting: token)
        // A sign-out that began while the refresh ran must not hand its token to the caller.
        return generation == signOuts ? refreshed : nil
    }

    /// Like `token()`, but nil unless the credentials were issued by `server`.
    public func token(for server: URL) async throws -> String? {
        try await bound(to: server) { try await token() }
    }

    /// Like `refresh(rejecting:)`, but nil unless the credentials were issued by `server`.
    public func refresh(rejecting token: String?, for server: URL) async throws -> String? {
        try await bound(to: server) { try await refresh(rejecting: token) }
    }

    /// Checks before, so another server can't trigger a refresh, and after, in case a sign-in replaced the credentials.
    private func bound(to server: URL, _ body: () async throws -> String?) async throws -> String? {
        guard try isBound(to: server) else { return nil }
        let token = try await body()
        return try isBound(to: server) ? token : nil
    }

    private func isBound(to server: URL) throws -> Bool {
        try (pendingSave ?? store.load())?.serverURL == server
    }

    private func sharedRefresh(rejecting token: String?) async throws -> String? {
        if let inFlight { return try await inFlight.value }
        let task = Task {
            if let refreshLock {
                return try await refreshLock.withLock { try await self.performRefresh(rejecting: token) }
            }
            return try await self.performRefresh(rejecting: token)
        }
        inFlight = task
        defer { inFlight = nil }
        do { return try await task.value } catch {
            changed()
            throw error
        }
    }

    /// Retries persisting a previously rotated token, if one is pending.
    public func retryPendingSave() async throws {
        guard !signingOut else { return }
        if let inFlight { _ = try? await inFlight.value; return }
        guard pendingSave != nil else { return }
        let task = Task {
            if let refreshLock {
                return try await refreshLock.withLock {
                    try await self.performRefresh(rejecting: nil, retryOnly: true)
                }
            }
            return try await self.performRefresh(rejecting: nil, retryOnly: true)
        }
        inFlight = task
        defer { inFlight = nil }
        _ = try await task.value
    }

    var isSigningOut: Bool { signingOut }

    public func signOut() async throws {
        guard !signingOut else { return }
        signingOut = true
        signOuts += 1
        defer { signingOut = false }
        if let inFlight { _ = try? await inFlight.value }
        if let refreshLock {
            try await refreshLock.withLock { try await self.clearCredentials() }
        } else {
            try await clearCredentials()
        }
    }

    private func clearCredentials() async throws {
        let credentials = try store.load()
        try store.clear()
        pendingSave = nil
        pendingBaseline = nil
        changed()
        if let credentials, let endpoint = credentials.revocationEndpoint,
            let refresh = credentials.tokens.refreshToken
        {
            await client.revoke(
                OAuthRequests.revocation(endpoint: endpoint, clientID: credentials.clientID, token: refresh))
        }
    }

    private func performRefresh(rejecting token: String?, retryOnly: Bool = false) async throws -> String? {
        if let pendingSave {
            switch try persist(pendingSave, replacing: pendingBaseline) {
            case .saved:
                changed()
                return pendingSave.isSignedIn ? pendingSave.tokens.accessToken : nil
            case let .superseded(current):
                if let token = usableToken(current) { return token }
            }
        }
        if retryOnly { return nil }
        guard let credentials = try store.load(), credentials.isSignedIn else { return nil }
        guard credentials.refreshPending != true else { throw FolioError.signInRequired }
        // Re-read after acquiring the process lock: an extension may have rotated the token.
        if let token, credentials.tokens.accessToken != token, let rotated = usableToken(credentials) {
            return rotated
        }
        guard let refreshToken = credentials.tokens.refreshToken else {
            var signedOut = credentials
            signedOut.tokens = .signedOut
            try persist(signedOut, replacing: credentials)
            changed()
            return nil
        }
        let request = OAuthRequests.refresh(
            endpoint: credentials.tokenEndpoint, clientID: credentials.clientID, refreshToken: refreshToken)
        var updated = credentials
        // A lost response or failed save must never let another process replay a consumed token.
        updated.refreshPending = true
        try store.save(updated)
        let baseline = updated
        do {
            updated.tokens = try await client.tokens(for: request, keeping: refreshToken)
        } catch OAuthFailure.unreachable {
            try persist(credentials, replacing: baseline)
            throw FolioError.unreachable
        } catch OAuthFailure.invalidGrant {
            updated.tokens = .signedOut
            updated.refreshPending = nil
            try persist(updated, replacing: baseline)
            changed()
            return nil
        }
        updated.refreshPending = nil
        if case let .superseded(current) = try persist(updated, replacing: baseline) { return usableToken(current) }
        return updated.tokens.accessToken
    }

    private enum Write {
        case saved
        case superseded(by: OAuthCredentials?)
    }

    /// Saves `updated` unless another writer, such as a newer sign-in, replaced `baseline` meanwhile.
    /// A failed save keeps it pending, so `retryPendingSave()` can write it later.
    @discardableResult
    private func persist(_ updated: OAuthCredentials, replacing baseline: OAuthCredentials?) throws -> Write {
        pendingSave = updated
        pendingBaseline = baseline
        let current = try store.load()
        let unchanged = current == baseline
        if unchanged { try store.save(updated) }
        pendingSave = nil
        pendingBaseline = nil
        return unchanged ? .saved : .superseded(by: current)
    }

    private func usableToken(_ credentials: OAuthCredentials?) -> String? {
        guard let credentials, credentials.isSignedIn, credentials.refreshPending != true,
            credentials.tokens.expiresAt.timeIntervalSince(now()) > leeway
        else { return nil }
        return credentials.tokens.accessToken
    }

    private func changed() {
        NotificationCenter.default.post(name: Notification.Name("FolioSessionChanged"), object: nil)
    }
}

/// Hands out an `OAuthSession`'s tokens only to the server that issued them.
public struct ServerBoundTokenProvider: BearerTokenProvider {
    private let session: OAuthSession
    private let server: URL

    public init(session: OAuthSession, server: URL) {
        self.session = session
        self.server = server
    }

    public func token() async throws -> String? {
        try await session.token(for: server)
    }

    public func refresh(rejecting token: String?) async throws -> String? {
        try await session.refresh(rejecting: token, for: server)
    }
}
