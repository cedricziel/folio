import FileProvider
import Foundation
import Synchronization

/// What this device records about an item: when it was last used (Recents), its Finder tags, its favorite
/// rank and whether to keep it downloaded. Folio has no server field for any of them.
struct LocalItemMetadata: Codable, Equatable, Sendable {
    var lastUsedDate: Date?
    var tagData: Data?
    var favoriteRank: UInt64?
    var keepDownloaded: Bool?

    // Favorite ranks exist only in the iOS Files app; macOS marks the field unavailable.
    #if os(macOS)
        static let fields: NSFileProviderItemFields = [.lastUsedDate, .tagData]
    #else
        static let fields: NSFileProviderItemFields = [.lastUsedDate, .tagData, .favoriteRank]
    #endif
}

struct LocalItemMetadataEdit: Sendable {
    private let fields: NSFileProviderItemFields
    private let values: LocalItemMetadata

    init(item: NSFileProviderItem, fields: NSFileProviderItemFields) {
        self.fields = fields.intersection(LocalItemMetadata.fields)
        var values = LocalItemMetadata()
        if fields.contains(.lastUsedDate) {
            values.lastUsedDate = item.lastUsedDate ?? nil
        }
        if fields.contains(.tagData) {
            values.tagData = item.tagData ?? nil
        }
        #if !os(macOS)
            if fields.contains(.favoriteRank) {
                values.favoriteRank = (item.favoriteRank ?? nil)?.uint64Value
            }
        #endif
        self.values = values
    }

    func apply(to metadata: inout LocalItemMetadata) {
        if fields.contains(.lastUsedDate) {
            metadata.lastUsedDate = values.lastUsedDate
        }
        if fields.contains(.tagData) {
            metadata.tagData = values.tagData
        }
        #if !os(macOS)
            if fields.contains(.favoriteRank) {
                metadata.favoriteRank = values.favoriteRank
            }
        #endif
    }
}

/// Keeps ``LocalItemMetadata`` per item identifier in a JSON file beside the extension. Every item the
/// extension reports must carry it, because the system treats a missing tag or rank as cleared.
final class LocalItemMetadataStore: Sendable {
    private let file: URL
    private let entries: Mutex<[String: LocalItemMetadata]>

    init(file: URL) {
        self.file = file
        entries = Mutex(
            (try? JSONDecoder().decode([String: LocalItemMetadata].self, from: Data(contentsOf: file))) ?? [:]
        )
    }

    private static let sharedStores = Mutex<[URL: LocalItemMetadataStore]>([:])

    /// One store per file, so extension instances that overlap don't overwrite each other's writes.
    static func shared(file: URL) -> LocalItemMetadataStore {
        sharedStores.withLock { stores in
            if let store = stores[file] {
                return store
            }
            let store = LocalItemMetadataStore(file: file)
            stores[file] = store
            return store
        }
    }

    /// A store in a fresh temporary file, for tests and for when Application Support is unavailable.
    static func scratch() -> LocalItemMetadataStore {
        LocalItemMetadataStore(
            file: FileManager.default.temporaryDirectory.appendingPathComponent(
                "folio-local-metadata-\(UUID().uuidString).json"
            )
        )
    }

    static func file(domain: NSFileProviderDomain) throws -> URL {
        try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        .appendingPathComponent("FolioLocalMetadata", isDirectory: true)
        .appendingPathComponent(domain.identifier.rawValue + ".json")
    }

    func metadata(for id: NSFileProviderItemIdentifier) -> LocalItemMetadata {
        entries.withLock { $0[id.rawValue] ?? LocalItemMetadata() }
    }

    func update(with edit: LocalItemMetadataEdit, for id: NSFileProviderItemIdentifier) throws -> LocalItemMetadata {
        try entries.withLock { entries in
            let key = id.rawValue
            var metadata = entries[key] ?? LocalItemMetadata()
            edit.apply(to: &metadata)
            try save([key: metadata], to: &entries)
            return metadata
        }
    }

    func setKeepDownloaded(_ keep: Bool, for ids: [NSFileProviderItemIdentifier]) throws {
        try entries.withLock { entries in
            var changed: [String: LocalItemMetadata] = [:]
            for id in ids {
                var metadata = entries[id.rawValue] ?? LocalItemMetadata()
                metadata.keepDownloaded = keep ? true : nil
                changed[id.rawValue] = metadata
            }
            try save(changed, to: &entries)
        }
    }

    /// Write the file first so a failure leaves the current entries unchanged.
    private func save(_ changed: [String: LocalItemMetadata], to entries: inout [String: LocalItemMetadata]) throws {
        var updated = entries
        for (key, metadata) in changed {
            updated[key] = metadata == LocalItemMetadata() ? nil : metadata
        }
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(updated).write(to: file, options: .atomic)
        entries = updated
    }
}
