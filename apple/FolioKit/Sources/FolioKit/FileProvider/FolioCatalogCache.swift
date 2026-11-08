import CryptoKit
import Foundation
import os

/// Recent vault and folder listings for one File Provider process.
///
/// Files and Finder resolve items one at a time, and each item is found in its folder's listing; without
/// reuse, every lookup downloaded the folder's whole document list again. A caller shares a request already
/// in flight only if that request started recently enough for the caller's `maxAge`, so a fresh enumeration
/// never gets data requested before it, such as a listing from before an upload.
public actor FolioCatalogCache {
    /// A value and when the request that produced it started.
    private struct Entry<Value: Sendable>: Sendable {
        let value: Value
        let started: ContinuousClock.Instant
    }

    private struct Request<Value: Sendable> {
        let task: Task<Value, Error>
        let started: ContinuousClock.Instant
    }

    private let clock = ContinuousClock()
    private var vaultEntry: Entry<VaultCatalog>?
    private var vaultRequest: Request<VaultCatalog>?
    private var listings: [FolioItemID.Folder: Entry<[FolioNode]>] = [:]
    private var listingRequests: [FolioItemID.Folder: Request<[FolioNode]>] = [:]
    /// Bumped by ``invalidateListings()``; responses to requests from an earlier generation are not stored.
    private var generation = 0

    /// No caller reuses a listing older than this, so older ones are dropped whenever a listing is stored.
    private static let retention: Duration = .seconds(60)

    public init() {}

    /// Returns a catalog requested within `maxAge`, else fetches one; `.zero` always starts a new request.
    func vaults(
        maxAge: Duration, fetch: @escaping @Sendable () async throws -> VaultCatalog
    ) async throws -> VaultCatalog {
        let now = clock.now
        if let vaultEntry, now - vaultEntry.started < maxAge { return vaultEntry.value }
        if let vaultRequest, now - vaultRequest.started < maxAge { return try await vaultRequest.task.value }
        let request = Request(task: Task { try await fetch() }, started: now)
        vaultRequest = request
        defer { if vaultRequest?.started == request.started { vaultRequest = nil } }
        let value = try await request.task.value
        if (vaultEntry?.started).map({ $0 <= request.started }) ?? true {
            vaultEntry = Entry(value: value, started: request.started)
        }
        return value
    }

    /// Returns `folder`'s listing requested within `maxAge`, else fetches it; `.zero` always starts a new
    /// request.
    func children(
        of folder: FolioItemID.Folder, maxAge: Duration,
        fetch: @escaping @Sendable () async throws -> [FolioNode]
    ) async throws -> [FolioNode] {
        let now = clock.now
        if let entry = listings[folder], now - entry.started < maxAge { return entry.value }
        if let request = listingRequests[folder], now - request.started < maxAge {
            return try await request.task.value
        }
        let request = Request(task: Task { try await fetch() }, started: now)
        let requestGeneration = generation
        listingRequests[folder] = request
        defer { if listingRequests[folder]?.started == request.started { listingRequests[folder] = nil } }
        let value = try await request.task.value
        guard generation == requestGeneration else { return value }
        if (listings[folder]?.started).map({ $0 <= request.started }) ?? true {
            listings[folder] = Entry(value: value, started: request.started)
        }
        let cutoff = clock.now
        listings = listings.filter { cutoff - $0.value.started < Self.retention }
        return value
    }

    /// Forgets every folder listing, for example after an upload added a document to some of them.
    /// Requests already in flight finish for their callers but are not stored.
    func invalidateListings() {
        generation += 1
        listings.removeAll()
        listingRequests.removeAll()
    }
}

/// One catalog cache per sign-in for a whole File Provider process, shared by every provider instance.
///
/// The key is the server and a digest of the stored refresh token. Signing in again, as any account, starts an
/// empty cache, so the extension never serves the previous account's listings. A token refresh does too, which
/// costs one refetch. Signed out, nothing is kept.
public final class FolioCatalogCaches: Sendable {
    private struct Key: Equatable {
        let server: URL
        let signIn: String
    }

    private let current = OSAllocatedUnfairLock<(key: Key, cache: FolioCatalogCache)?>(initialState: nil)

    public init() {}

    /// The cache for `credentials` on `server`, or nil when they are signed out or belong to another server.
    public func cache(for server: URL, credentials: OAuthCredentials?) -> FolioCatalogCache? {
        guard let credentials, credentials.isSignedIn, credentials.serverURL == server else {
            clear()
            return nil
        }
        let token = credentials.tokens.refreshToken ?? credentials.tokens.accessToken
        let digest = SHA256.hash(data: Data(token.utf8)).map { String(format: "%02x", $0) }.joined()
        let key = Key(server: server, signIn: digest)
        return current.withLock { current in
            if let current, current.key == key { return current.cache }
            let cache = FolioCatalogCache()
            current = (key, cache)
            return cache
        }
    }

    public func clear() {
        current.withLock { $0 = nil }
    }
}
