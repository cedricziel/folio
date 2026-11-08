import FileProvider
import Foundation

/// What this device records about an item: when it was last used (Recents), its Finder tags, its favorite
/// rank and whether to keep it downloaded. Folio has no server field for any of them.
struct LocalItemMetadata: Codable, Equatable, Sendable {
    var lastUsedDate: Date?
    var tagData: Data?
    var favoriteRank: UInt64?
    var keepDownloaded: Bool?

    /// Favorite ranks exist only in the iOS Files app; macOS marks the field unavailable.
    #if os(macOS)
        static let fields: NSFileProviderItemFields = [.lastUsedDate, .tagData]
    #else
        static let fields: NSFileProviderItemFields = [.lastUsedDate, .tagData, .favoriteRank]
    #endif
}

/// Keeps ``LocalItemMetadata`` per item identifier in a JSON file beside the extension. Every item the
/// extension reports must carry it, because the system treats a missing tag or rank as cleared.
final class LocalItemMetadataStore: @unchecked Sendable {
    private let file: URL
    private let lock = NSLock()
    private var entries: [String: LocalItemMetadata]

    init(file: URL) {
        self.file = file
        entries = (try? JSONDecoder().decode([String: LocalItemMetadata].self, from: Data(contentsOf: file))) ?? [:]
    }

    private static let sharedLock = NSLock()
    nonisolated(unsafe) private static var sharedStores: [URL: LocalItemMetadataStore] = [:]

    /// One store per file, so extension instances that overlap don't overwrite each other's writes.
    static func shared(file: URL) -> LocalItemMetadataStore {
        sharedLock.withLock {
            if let store = sharedStores[file] { return store }
            let store = LocalItemMetadataStore(file: file)
            sharedStores[file] = store
            return store
        }
    }

    /// A store in a fresh temporary file, for tests and for when Application Support is unavailable.
    static func scratch() -> LocalItemMetadataStore {
        LocalItemMetadataStore(
            file: FileManager.default.temporaryDirectory.appendingPathComponent(
                "folio-local-metadata-\(UUID().uuidString).json"))
    }

    static func file(domain: NSFileProviderDomain) throws -> URL {
        try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        .appendingPathComponent("FolioLocalMetadata", isDirectory: true)
        .appendingPathComponent(domain.identifier.rawValue + ".json")
    }

    func metadata(for id: NSFileProviderItemIdentifier) -> LocalItemMetadata {
        lock.withLock { entries[id.rawValue] ?? LocalItemMetadata() }
    }

    /// Copies the local `fields` from `item` to the entry for `id` (the item's own identifier by default)
    /// and saves them before returning the result.
    func update(
        with item: NSFileProviderItem, fields: NSFileProviderItemFields, for id: NSFileProviderItemIdentifier? = nil
    ) throws -> LocalItemMetadata {
        let fields = fields.intersection(LocalItemMetadata.fields)
        return try lock.withLock {
            let key = (id ?? item.itemIdentifier).rawValue
            var metadata = entries[key] ?? LocalItemMetadata()
            if fields.contains(.lastUsedDate) { metadata.lastUsedDate = item.lastUsedDate ?? nil }
            if fields.contains(.tagData) { metadata.tagData = item.tagData ?? nil }
            #if !os(macOS)
                if fields.contains(.favoriteRank) { metadata.favoriteRank = (item.favoriteRank ?? nil)?.uint64Value }
            #endif
            try save([key: metadata])
            return metadata
        }
    }

    func setKeepDownloaded(_ keep: Bool, for ids: [NSFileProviderItemIdentifier]) throws {
        try lock.withLock {
            var changed: [String: LocalItemMetadata] = [:]
            for id in ids {
                var metadata = entries[id.rawValue] ?? LocalItemMetadata()
                metadata.keepDownloaded = keep ? true : nil
                changed[id.rawValue] = metadata
            }
            try save(changed)
        }
    }

    /// Callers hold the lock. Writes the file first, so a failed write leaves memory unchanged.
    private func save(_ changed: [String: LocalItemMetadata]) throws {
        var updated = entries
        for (key, metadata) in changed {
            updated[key] = metadata == LocalItemMetadata() ? nil : metadata
        }
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(updated).write(to: file, options: .atomic)
        entries = updated
    }
}
