import Foundation

/// Recent vault and folder listings for one File Provider process.
///
/// Files and Finder resolve items one at a time, and each item is found in its folder's listing; without
/// reuse, every lookup downloaded the folder's whole document list again. Requests already in flight are
/// shared, so a burst of lookups costs one listing.
public actor FolioCatalogCache {
    private struct Entry<Value: Sendable>: Sendable {
        let value: Value
        let fetched: ContinuousClock.Instant
    }

    private let clock = ContinuousClock()
    private var vaultEntry: Entry<VaultCatalog>?
    private var vaultRequest: Task<VaultCatalog, Error>?
    private var listings: [FolioItemID.Folder: Entry<[FolioNode]>] = [:]
    private var listingRequests: [FolioItemID.Folder: Task<[FolioNode], Error>] = [:]

    public init() {}

    /// Returns a catalog fetched within `maxAge`, else fetches one; `.zero` always fetches.
    func vaults(
        maxAge: Duration, fetch: @escaping @Sendable () async throws -> VaultCatalog
    ) async throws -> VaultCatalog {
        if let vaultEntry, clock.now - vaultEntry.fetched < maxAge { return vaultEntry.value }
        if let vaultRequest { return try await vaultRequest.value }
        let request = Task { try await fetch() }
        vaultRequest = request
        defer { vaultRequest = nil }
        let value = try await request.value
        vaultEntry = Entry(value: value, fetched: clock.now)
        return value
    }

    /// Returns `folder`'s listing fetched within `maxAge`, else fetches it; `.zero` always fetches.
    func children(
        of folder: FolioItemID.Folder, maxAge: Duration,
        fetch: @escaping @Sendable () async throws -> [FolioNode]
    ) async throws -> [FolioNode] {
        if let entry = listings[folder], clock.now - entry.fetched < maxAge { return entry.value }
        if let request = listingRequests[folder] { return try await request.value }
        let request = Task { try await fetch() }
        listingRequests[folder] = request
        defer { listingRequests[folder] = nil }
        let value = try await request.value
        listings[folder] = Entry(value: value, fetched: clock.now)
        return value
    }
}
