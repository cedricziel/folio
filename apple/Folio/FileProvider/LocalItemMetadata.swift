import FileProvider
import Foundation

/// What this device records about an item: when it was last used (Recents), its Finder tags, its favorite
/// rank and whether to keep it downloaded. Folio has no server field for any of them.
struct LocalItemMetadata: Codable, Equatable, Sendable {
    var lastUsedDate: Date?
    var tagData: Data?
    var favoriteRank: UInt64?
    var keepDownloaded: Bool?

    static let fields: NSFileProviderItemFields = [.lastUsedDate, .tagData, .favoriteRank]
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

    /// Copies the `fields` that changed from `item` and saves them before returning the result.
    func update(with item: NSFileProviderItem, fields: NSFileProviderItemFields) throws -> LocalItemMetadata {
        try lock.withLock {
            let key = item.itemIdentifier.rawValue
            var metadata = entries[key] ?? LocalItemMetadata()
            if fields.contains(.lastUsedDate) { metadata.lastUsedDate = item.lastUsedDate ?? nil }
            if fields.contains(.tagData) { metadata.tagData = item.tagData ?? nil }
            if fields.contains(.favoriteRank) { metadata.favoriteRank = (item.favoriteRank ?? nil)?.uint64Value }
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
