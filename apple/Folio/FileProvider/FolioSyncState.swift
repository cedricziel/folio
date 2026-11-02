import CryptoKit
import FileProvider
import FolioKit

actor FolioSyncState {
    struct Snapshot {
        let nodes: [FolioNode]
        let versions: [String: Data]
        let anchor: NSFileProviderSyncAnchor
    }

    private let catalog: FolioCatalog
    private let container: FolioItemID.Folder?
    private let directory: URL
    private var initial: Task<Snapshot, Error>?

    init(catalog: FolioCatalog, container: FolioItemID.Folder?, directory: URL) {
        self.catalog = catalog
        self.container = container
        self.directory = directory
    }

    static func directory(domain: NSFileProviderDomain, container: NSFileProviderItemIdentifier) throws -> URL {
        let scope = [
            domain.identifier.rawValue, SharedConfiguration.serverURL()?.absoluteString ?? "", container.rawValue,
        ]
        let key = SHA256.hash(data: Data(scope.joined(separator: "\n").utf8)).map { String(format: "%02x", $0) }
            .joined()
        return try FileManager.default.url(
            for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        .appendingPathComponent("FolioSync", isDirectory: true).appendingPathComponent(key, isDirectory: true)
    }

    static func metadataVersion(_ node: FolioNode) -> Data {
        let fields = [
            node.id.rawValue, node.parent.rawValue, node.name, node.contentType ?? "",
            String(node.isFolder), String(node.modified.timeIntervalSince1970),
            String(node.document?.createdAt.timeIntervalSince1970 ?? 0),
            String(node.canUpload),
        ]
        let encoded = fields.map { "\($0.utf8.count):\($0)" }.joined()
        return Data(SHA256.hash(data: Data(encoded.utf8)))
    }

    func initialSnapshot() async throws -> Snapshot {
        if let initial {
            return try await initial.value
        }
        let task = Task { try await capture() }
        initial = task
        do {
            return try await task.value
        } catch {
            initial = nil
            throw error
        }
    }

    func changes(from anchor: NSFileProviderSyncAnchor) async throws -> (
        Snapshot, [FolioNode], [NSFileProviderItemIdentifier]
    ) {
        guard let key = String(data: anchor.rawValue, encoding: .utf8), key.count == 64,
            key.allSatisfy({ $0.isHexDigit && !$0.isUppercase }),
            let data = try? Data(contentsOf: directory.appendingPathComponent(key)),
            snapshotKey(data) == key,
            let previous = try? JSONDecoder().decode([String: Data].self, from: data)
        else { throw NSFileProviderError(.syncAnchorExpired) }
        let current = try await capture()
        let updated = current.nodes.filter { previous[$0.id.rawValue] != current.versions[$0.id.rawValue] }
        let deleted = previous.keys.filter { current.versions[$0] == nil }.sorted().map {
            NSFileProviderItemIdentifier(rawValue: $0)
        }
        initial = Task { current }
        return (current, updated, deleted)
    }

    private func capture() async throws -> Snapshot {
        let nodes: [FolioNode]
        if let container {
            nodes = try await catalog.children(of: container)
        } else {
            nodes = try await catalog.allNodes()
        }
        let versions = Dictionary(uniqueKeysWithValues: nodes.map { ($0.id.rawValue, Self.metadataVersion($0)) })
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let data = try encoder.encode(versions)
        let key = snapshotKey(data)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try data.write(to: directory.appendingPathComponent(key), options: .atomic)
        prune(keeping: key)
        return Snapshot(nodes: nodes, versions: versions, anchor: NSFileProviderSyncAnchor(Data(key.utf8)))
    }

    private func snapshotKey(_ data: Data) -> String {
        let scope = Data((container.map { FolioItemID.folder($0).rawValue } ?? "working-set").utf8)
        return SHA256.hash(data: scope + data).map { String(format: "%02x", $0) }.joined()
    }

    private func prune(keeping key: String) {
        let files =
            (try? FileManager.default.contentsOfDirectory(
                at: directory, includingPropertiesForKeys: [.contentModificationDateKey]
            )) ?? []
        let oldest = files.filter { $0.lastPathComponent != key }.sorted {
            let left = try? $0.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
            let right = try? $1.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
            return (left ?? .distantPast) < (right ?? .distantPast)
        }
        for file in oldest.prefix(max(0, files.count - 32)) {
            try? FileManager.default.removeItem(at: file)
        }
    }
}
