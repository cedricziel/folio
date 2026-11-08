import Foundation
import os

/// Files handed to other apps: a few recent downloads, plus previews the app already holds. Downloads are
/// moved into one folder and removed once they are old or the folder holds too many; everything is
/// forgotten, and the downloads deleted, when the session ends.
public final class TransferFileCache: Sendable {
    public static let shared = TransferFileCache(
        directory: FileManager.default.temporaryDirectory.appending(
            path: "folio-transfers", directoryHint: .isDirectory)
    )

    private struct Entry {
        let url: URL
        let owned: Bool
        let stored: Date
    }

    private let directory: URL
    private let maxAge: TimeInterval
    private let maxFiles: Int
    private let now: @Sendable () -> Date
    private let entries = OSAllocatedUnfairLock(initialState: [DocumentEntityID: Entry]())
    private let server = OSAllocatedUnfairLock<URL?>(initialState: nil)
    private let swept = OSAllocatedUnfairLock(initialState: false)

    public init(
        directory: URL, maxAge: TimeInterval = 600, maxFiles: Int = 8, now: @escaping @Sendable () -> Date = Date.init
    ) {
        self.directory = directory
        self.maxAge = maxAge
        self.maxFiles = maxFiles
        self.now = now
    }

    /// Offers a file the app already downloaded for this document, such as the open preview. The cache
    /// never deletes it.
    public func adopt(_ url: URL, for id: DocumentEntityID) {
        entries.withLock { $0[id] = Entry(url: url, owned: false, stored: now()) }
    }

    /// A copy still on disk, if one is known.
    public func file(for id: DocumentEntityID) -> URL? {
        prune()
        guard let entry = entries.withLock({ $0[id] }) else { return nil }
        guard FileManager.default.fileExists(atPath: entry.url.path) else {
            entries.withLock { _ = $0.removeValue(forKey: id) }
            return nil
        }
        return entry.url
    }

    /// Moves a fresh download into the cache, keeping its filename, and removes the folder it came in.
    public func store(_ download: URL, for id: DocumentEntityID) throws -> URL {
        let folder = directory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let destination = folder.appending(path: download.lastPathComponent)
        try FileManager.default.moveItem(at: download, to: destination)
        let source = download.deletingLastPathComponent()
        if source.lastPathComponent.hasPrefix("folio-"),
            (try? FileManager.default.contentsOfDirectory(atPath: source.path))?.isEmpty == true
        {
            try? FileManager.default.removeItem(at: source)
        }
        let replaced = entries.withLock { entries in
            let old = entries[id]
            entries[id] = Entry(url: destination, owned: true, stored: now())
            return old
        }
        if let replaced, replaced.owned { remove(replaced.url) }
        prune()
        return destination
    }

    /// Deletes downloads past their age, then the oldest beyond the limit.
    public func prune() {
        let cutoff = now().addingTimeInterval(-maxAge)
        let expired = entries.withLock { entries in
            var expired: [Entry] = []
            for (id, entry) in entries where entry.stored < cutoff {
                expired.append(entry)
                entries.removeValue(forKey: id)
            }
            let owned = entries.filter { $0.value.owned }.sorted { $0.value.stored > $1.value.stored }
            for (id, entry) in owned.dropFirst(maxFiles) {
                expired.append(entry)
                entries.removeValue(forKey: id)
            }
            return expired
        }
        for entry in expired where entry.owned { remove(entry.url) }
    }

    /// Forgets every file and deletes every download.
    public func removeAll() {
        entries.withLock { $0.removeAll() }
        try? FileManager.default.removeItem(at: directory)
    }

    /// Keeps files only while one server stays signed in; a sign-out, an expired session or another server
    /// deletes them. Interruptions keep them, as the same session may come back.
    public func sessionChanged(to session: SessionRestoration) {
        sweepLeftoversOnce()
        switch session {
        case let .signedIn(url):
            let changed = server.withLock { server in
                defer { server = url }
                return server != nil && server != url
            }
            if changed { removeAll() }
        case .signedOut, .expired:
            server.withLock { $0 = nil }
            removeAll()
        case .restoring, .interrupted, .unavailable:
            return
        }
    }

    /// An interactive sign-in may be another account on the same server, so it starts empty.
    public func signedIn(to url: URL) {
        server.withLock { $0 = url }
        removeAll()
    }

    /// Downloads from earlier launches aren't tracked in memory, so the first session update of a launch
    /// applies the age and count limits to the folders they left behind.
    private func sweepLeftoversOnce() {
        let first = swept.withLock { swept in
            defer { swept = true }
            return !swept
        }
        guard first else { return }
        let tracked = entries.withLock { Set($0.values.map { $0.url.deletingLastPathComponent().standardizedFileURL }) }
        let folders =
            (try? FileManager.default.contentsOfDirectory(
                at: directory, includingPropertiesForKeys: [.contentModificationDateKey]))?
            .filter { !tracked.contains($0.standardizedFileURL) } ?? []
        let cutoff = now().addingTimeInterval(-maxAge)
        let dated = folders.map { folder in
            (folder, (try? folder.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
                ?? .distantPast)
        }
        .sorted { $0.1 > $1.1 }
        for (index, (folder, modified)) in dated.enumerated() where modified < cutoff || index >= maxFiles {
            try? FileManager.default.removeItem(at: folder)
        }
    }

    private func remove(_ url: URL) {
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    }
}
