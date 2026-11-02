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
    public var canUpload = false
}

/// Maps the server's data onto the virtual folder tree shown in Files and Finder.
public struct FolioCatalog: Sendable {
    private let service: any FolioService

    public init(service: any FolioService) {
        self.service = service
    }

    private struct Context {
        let service: any FolioService
        let vault: Vault?

        func folder(_ folder: FolioItemID.Folder) -> FolioItemID.Folder {
            guard let vault else { return folder }
            return folder == .root ? .vault(vault.id) : .scoped(vault.id, folder)
        }

        func folderNode(
            _ folder: FolioItemID.Folder, _ name: String, parent: FolioItemID.Folder = .root
        ) -> FolioNode {
            FolioNode(
                id: .folder(self.folder(folder)), parent: .folder(self.folder(parent)),
                name: FolioCatalog.sanitized(name), isFolder: true, modified: .distantPast,
                contentType: nil, document: nil,
                canUpload: folder.acceptsUploads && (vault?.role.canUpload ?? true)
            )
        }
    }

    private func context(for folder: FolioItemID.Folder) async throws -> Context? {
        let catalog = try await service.vaults()
        if let id = folder.vaultID {
            guard let vault = catalog.vaults.first(where: { $0.id == id }) else { return nil }
            return Context(service: service.scoped(to: id), vault: vault)
        }
        guard catalog == .legacy else { return nil }
        return Context(service: service, vault: nil)
    }

    public func children(of folder: FolioItemID.Folder) async throws -> [FolioNode] {
        if folder == .root {
            let catalog = try await service.vaults()
            if catalog != .legacy {
                return vaultNodes(catalog.vaults)
            }
            return try await children(of: .root, context: Context(service: service, vault: nil))
        }
        guard let context = try await context(for: folder) else { return [] }
        let content: FolioItemID.Folder
        if case .vault = folder {
            content = .root
        } else {
            content = folder.contentFolder
        }
        do {
            return try await children(of: content, context: context)
        } catch FolioError.server(status: 404, message: _) {
            return []
        }
    }

    public func node(for id: FolioItemID) async throws -> FolioNode? {
        switch id {
        case .folder(.root):
            return FolioNode(
                id: id, parent: id, name: "Folio", isFolder: true, modified: .distantPast,
                contentType: nil, document: nil
            )
        case let .folder(folder):
            guard let parent = folder.parent else { return nil }
            return try await children(of: parent).first { $0.id == id }
        case let .document(_, folder):
            return try await children(of: folder).first { $0.id == id }
        }
    }

    public func upload(_ file: UploadFile, to folder: FolioItemID.Folder = .inbox) async throws -> FolioNode {
        guard folder.acceptsUploads else { throw FolioError.server(status: 403, message: nil) }
        guard let context = try await context(for: folder) else { throw FolioError.server(status: 404, message: nil) }
        guard context.vault?.role.canUpload != false else { throw FolioError.server(status: 403, message: nil) }
        let document = try await context.service.upload(file)
        return documentNodes([document], in: folder)[0]
    }

    public func downloadFile(for node: FolioNode) async throws -> URL {
        guard let document = node.document, case let .folder(folder) = node.parent,
            let context = try await context(for: folder)
        else { throw FolioError.server(status: 404, message: nil) }
        return try await context.service.downloadFile(for: document)
    }

    public func allNodes() async throws -> [FolioNode] {
        let catalog = try await service.vaults()
        if catalog == .legacy {
            return try await allNodes(context: Context(service: service, vault: nil))
        }
        let roots = vaultNodes(catalog.vaults)
        var nodes: [FolioNode] = []
        for vault in catalog.vaults.sorted(by: { $0.id.uuidString < $1.id.uuidString }) {
            do {
                let scoped = try await allNodes(context: Context(service: service.scoped(to: vault.id), vault: vault))
                nodes += roots.filter { $0.id == .folder(.vault(vault.id)) }
                nodes += scoped
            } catch FolioError.server(status: 404, message: _) {
                continue
            }
        }
        return nodes
    }

    private func vaultNodes(_ vaults: [Vault]) -> [FolioNode] {
        let names = Dictionary(grouping: vaults, by: { Self.sanitized($0.name).lowercased() })
        let reserved = Set(names.keys)
        return vaults.sorted { $0.id.uuidString < $1.id.uuidString }.map { vault in
            let name = Self.sanitized(vault.name)
            var unique = name
            if names[name.lowercased()]!.count > 1 {
                repeat {
                    unique += " (\(vault.id.uuidString.lowercased()))"
                } while reserved.contains(unique.lowercased())
            }
            return FolioNode(
                id: .folder(.vault(vault.id)), parent: .folder(.root), name: unique,
                isFolder: true, modified: .distantPast, contentType: nil, document: nil
            )
        }
    }

    private func children(of folder: FolioItemID.Folder, context: Context) async throws -> [FolioNode] {
        switch folder {
        case .root:
            return [
                context.folderNode(.inbox, "Inbox"), context.folderNode(.all, "All documents"),
                context.folderNode(.tags, "Tags"),
                context.folderNode(.correspondents, "Correspondents"), context.folderNode(.types, "Types"),
            ]
        case .inbox:
            let documents = try await context.service.documents(matching: "").filter { $0.status != .filed }
            return documentNodes(documents, in: context.folder(.inbox))
        case .all:
            return try documentNodes(await context.service.documents(matching: ""), in: context.folder(.all))
        case .tags:
            return try await context.service.tags().map { context.folderNode(.tag($0.id), $0.name, parent: .tags) }
        case .correspondents:
            return try await context.service.correspondents().map {
                context.folderNode(.correspondent($0.id), $0.name, parent: .correspondents)
            }
        case .types:
            return try await context.service.documentTypes().map {
                context.folderNode(.type($0.id), $0.name, parent: .types)
            }
        case let .correspondent(id):
            let documents = try await context.service.documents(filter: DocumentFilter(correspondentID: id))
            return documentNodes(documents, in: context.folder(folder))
        case let .type(id):
            let documents = try await context.service.documents(filter: DocumentFilter(documentTypeID: id))
            return documentNodes(documents, in: context.folder(folder))
        case .vault, .scoped:
            return []
        case let .tag(id):
            return try documentNodes(
                await context.service.documents(filter: DocumentFilter(tagIDs: [id])), in: context.folder(folder))
        }
    }

    private func allNodes(context: Context) async throws -> [FolioNode] {
        async let listed = context.service.documents(matching: "")
        async let people = context.service.correspondents()
        async let types = context.service.documentTypes()
        async let tags = context.service.tags()
        let (documents, correspondents, documentTypes, allTags) = try await (listed, people, types, tags)
        var nodes = try await children(of: .root, context: context)
        nodes += documentNodes(documents, in: context.folder(.all))
        nodes += documentNodes(documents.filter { $0.status != .filed }, in: context.folder(.inbox))
        for person in correspondents {
            nodes.append(context.folderNode(.correspondent(person.id), person.name, parent: .correspondents))
            nodes += documentNodes(
                documents.filter { $0.correspondentID == person.id }, in: context.folder(.correspondent(person.id)))
        }
        for type in documentTypes {
            nodes.append(context.folderNode(.type(type.id), type.name, parent: .types))
            nodes += documentNodes(
                documents.filter { $0.documentTypeID == type.id }, in: context.folder(.type(type.id)))
        }
        for tag in allTags {
            nodes.append(context.folderNode(.tag(tag.id), tag.name, parent: .tags))
            nodes += documentNodes(
                documents.filter { $0.tags.contains { $0.id == tag.id } }, in: context.folder(.tag(tag.id)))
        }
        return nodes
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
        return cleaned.isEmpty || cleaned == "." || cleaned == ".." ? "Untitled" : cleaned
    }

    private static func disambiguated(_ name: String, with id: UUID) -> String {
        let suffix = String(id.uuidString.prefix(4)).lowercased()
        let ext = (name as NSString).pathExtension
        let stem = (name as NSString).deletingPathExtension
        return ext.isEmpty ? "\(stem) (\(suffix))" : "\(stem) (\(suffix)).\(ext)"
    }
}
