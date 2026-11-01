import Foundation

/// One entry in the File Provider tree, independent of the FileProvider framework so it can be tested.
public struct FolioNode: Equatable, Sendable {
    public let id: FolioItemID
    public let parent: FolioItemID
    public let name: String
    public let isFolder: Bool
    public let modified: Date
    public let contentType: String?
    public let document: Document?
}

/// Maps the server's data onto the virtual folder tree shown in Files and Finder.
public struct FolioCatalog: Sendable {
    private let service: any FolioService

    public init(service: any FolioService) {
        self.service = service
    }

    public func children(of folder: FolioItemID.Folder) async throws -> [FolioNode] {
        switch folder {
        case .root:
            return [
                folderNode(.inbox, "Inbox"), folderNode(.all, "All documents"), folderNode(.tags, "Tags"),
                folderNode(.correspondents, "Correspondents"), folderNode(.types, "Types"),
            ]
        case .inbox:
            let documents = try await service.documents(matching: "").filter { $0.status == .needsReview }
            return documentNodes(documents, in: .inbox)
        case .all:
            return try documentNodes(await service.documents(matching: ""), in: .all)
        case .tags:
            return try await service.tags().map { folderNode(.tag($0.id), $0.name, parent: .tags) }
        case .correspondents:
            return try await service.correspondents().map {
                folderNode(.correspondent($0.id), $0.name, parent: .correspondents)
            }
        case .types:
            return try await service.documentTypes().map { folderNode(.type($0.id), $0.name, parent: .types) }
        case let .correspondent(id):
            let documents = try await service.documents(correspondentID: id, documentTypeID: nil)
            return documentNodes(documents, in: folder)
        case let .type(id):
            let documents = try await service.documents(correspondentID: nil, documentTypeID: id)
            return documentNodes(documents, in: folder)
        case .tag:
            // Documents carry no tags in the API response yet.
            return []
        }
    }

    public func node(for id: FolioItemID) async throws -> FolioNode? {
        switch id {
        case .folder(.root):
            return FolioNode(
                id: id, parent: id, name: "Folio", isFolder: true, modified: .distantPast, contentType: nil,
                document: nil
            )
        case let .folder(folder):
            guard let parent = folder.parent else { return nil }
            return try await children(of: parent).first { $0.id == id }
        case let .document(documentID, folder):
            return try await children(of: folder).first { $0.id == .document(documentID, in: folder) }
        }
    }

    /// Every document under All documents plus the virtual folders: what the working set tracks.
    public func allNodes() async throws -> [FolioNode] {
        async let documents = children(of: .all)
        async let people = children(of: .correspondents)
        async let types = children(of: .types)
        async let tags = children(of: .tags)
        return try await documents + people + types + tags
    }

    /// Changes whenever a document is added, removed or updated; the working set uses it as its sync anchor.
    public func changeToken() async throws -> String {
        let documents = try await service.documents(matching: "")
        let digest = documents.map { "\($0.id.uuidString)\($0.updatedAt.timeIntervalSince1970)" }
            .sorted().joined()
            .unicodeScalars.reduce(UInt64(5381)) { ($0 &* 33) &+ UInt64($1.value) }
        return "\(documents.count)-\(String(digest, radix: 16))"
    }

    private func folderNode(
        _ folder: FolioItemID.Folder, _ name: String, parent: FolioItemID.Folder = .root
    ) -> FolioNode {
        FolioNode(
            id: .folder(folder), parent: .folder(parent), name: Self.sanitized(name), isFolder: true,
            modified: .distantPast, contentType: nil, document: nil
        )
    }

    private func documentNodes(_ documents: [Document], in folder: FolioItemID.Folder) -> [FolioNode] {
        var seen = Set<String>()
        return documents.sorted { $0.id.uuidString < $1.id.uuidString }.map { document in
            var name = Self.sanitized(document.originalFilename ?? document.title)
            if !seen.insert(name.lowercased()).inserted {
                name = Self.disambiguated(name, with: document.id)
                seen.insert(name.lowercased())
            }
            return FolioNode(
                id: .document(document.id, in: folder), parent: .folder(folder), name: name, isFolder: false,
                modified: document.updatedAt, contentType: document.mimeType, document: document
            )
        }
    }

    private static func sanitized(_ name: String) -> String {
        let cleaned = name.replacingOccurrences(of: "/", with: "-").replacingOccurrences(of: ":", with: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return cleaned.isEmpty ? "Untitled" : cleaned
    }

    private static func disambiguated(_ name: String, with id: UUID) -> String {
        let suffix = String(id.uuidString.prefix(4)).lowercased()
        let ext = (name as NSString).pathExtension
        let stem = (name as NSString).deletingPathExtension
        return ext.isEmpty ? "\(stem) (\(suffix))" : "\(stem) (\(suffix)).\(ext)"
    }
}
