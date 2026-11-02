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
        if credentials.refreshPending == true { return try await refresh(rejecting: credentials.tokens.accessToken) }
        if credentials.tokens.expiresAt.timeIntervalSince(now()) > leeway {
            return credentials.tokens.accessToken
        }
        return try await refresh(rejecting: credentials.tokens.accessToken)
    }

    public func refresh(rejecting token: String?) async throws -> String? {
        guard !signingOut else { return nil }
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

    public func signOut() async throws {
        guard !signingOut else { return }
        signingOut = true
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
            let current = try store.load()
            if current == pendingBaseline {
                try store.save(pendingSave)
                self.pendingSave = nil
                pendingBaseline = nil
                changed()
                return pendingSave.isSignedIn ? pendingSave.tokens.accessToken : nil
            }
            self.pendingSave = nil
            pendingBaseline = nil
            if let current, current.isSignedIn,
                current.refreshPending != true,
                current.tokens.expiresAt.timeIntervalSince(now()) > leeway
            {
                return current.tokens.accessToken
            }
        }
        if retryOnly { return nil }
        guard let credentials = try store.load(), credentials.isSignedIn else { return nil }
        guard credentials.refreshPending != true else { throw FolioError.signInRequired }
        // Re-read after acquiring the process lock: an extension may have rotated the token.
        if let token, credentials.tokens.accessToken != token,
            credentials.tokens.expiresAt.timeIntervalSince(now()) > leeway
        {
            return credentials.tokens.accessToken
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
        try persist(updated, replacing: baseline)
        return updated.tokens.accessToken
    }

    private func persist(_ updated: OAuthCredentials, replacing baseline: OAuthCredentials) throws {
        pendingSave = updated
        pendingBaseline = baseline
        try store.save(updated)
        pendingSave = nil
        pendingBaseline = nil
    }

    private func changed() {
        NotificationCenter.default.post(name: Notification.Name("FolioSessionChanged"), object: nil)
    }
}
