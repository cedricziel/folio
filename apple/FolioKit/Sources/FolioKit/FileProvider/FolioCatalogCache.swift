import Foundation

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
        listingRequests[folder] = request
        defer { if listingRequests[folder]?.started == request.started { listingRequests[folder] = nil } }
        let value = try await request.task.value
        if (listings[folder]?.started).map({ $0 <= request.started }) ?? true {
            listings[folder] = Entry(value: value, started: request.started)
        }
        return value
    }
}
