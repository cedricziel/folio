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
            let documents = try await service.documents(matching: "").filter { $0.status != .filed }
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
            let documents = try await service.documents(filter: DocumentFilter(correspondentID: id))
            return documentNodes(documents, in: folder)
        case let .type(id):
            let documents = try await service.documents(filter: DocumentFilter(documentTypeID: id))
            return documentNodes(documents, in: folder)
        case let .tag(id):
            return try documentNodes(await service.documents(filter: DocumentFilter(tagIDs: [id])), in: folder)
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

    public func upload(_ file: UploadFile) async throws -> FolioNode {
        let document = try await service.upload(file)
        return documentNodes([document], in: .inbox)[0]
    }

    public func allNodes() async throws -> [FolioNode] {
        async let listed = service.documents(matching: "")
        async let people = service.correspondents()
        async let types = service.documentTypes()
        async let tags = service.tags()
        let (documents, correspondents, documentTypes, allTags) = try await (listed, people, types, tags)
        var nodes = try await children(of: .root)
        nodes += documentNodes(documents, in: .all)
        nodes += documentNodes(documents.filter { $0.status != .filed }, in: .inbox)
        for person in correspondents {
            nodes.append(folderNode(.correspondent(person.id), person.name, parent: .correspondents))
            nodes += documentNodes(documents.filter { $0.correspondentID == person.id }, in: .correspondent(person.id))
        }
        for type in documentTypes {
            nodes.append(folderNode(.type(type.id), type.name, parent: .types))
            nodes += documentNodes(documents.filter { $0.documentTypeID == type.id }, in: .type(type.id))
        }
        for tag in allTags {
            nodes.append(folderNode(.tag(tag.id), tag.name, parent: .tags))
            nodes += documentNodes(documents.filter { $0.tags.contains { $0.id == tag.id } }, in: .tag(tag.id))
        }
        return nodes
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
