import Foundation

/// Identifier of an item in the File Provider tree. A document can show up under several folders
/// (Inbox, All documents, a correspondent), and File Provider needs one parent per identifier,
/// so a document's identifier includes the folder it is listed in.
public enum FolioItemID: Hashable, Sendable {
    public enum Folder: Hashable, Sendable {
        case root, inbox, all, tags, correspondents, types
        case tag(UUID), correspondent(UUID), type(UUID)
        case vault(UUID)
        indirect case scoped(UUID, Folder)

        public var vaultID: UUID? {
            switch self {
            case let .vault(id), let .scoped(id, _): id
            default: nil
            }
        }

        public var contentFolder: Folder {
            if case let .scoped(_, folder) = self {
                return folder
            }
            return self
        }

        public var acceptsUploads: Bool {
            contentFolder == .inbox
        }

        public var parent: Folder? {
            switch self {
            case .root: nil
            case .inbox, .all, .tags, .correspondents, .types: .root
            case .tag: .tags
            case .correspondent: .correspondents
            case .type: .types
            case .vault: .root
            case let .scoped(id, folder):
                folder.parent.map { $0 == .root ? .vault(id) : .scoped(id, $0) }
            }
        }

        var rawValue: String {
            switch self {
            case .root: "root"
            case .inbox: "inbox"
            case .all: "all"
            case .tags: "tags"
            case .correspondents: "correspondents"
            case .types: "types"
            case let .tag(id): "tag:\(id.uuidString.lowercased())"
            case let .correspondent(id): "correspondent:\(id.uuidString.lowercased())"
            case let .type(id): "type:\(id.uuidString.lowercased())"
            case let .vault(id): "vault:\(id.uuidString.lowercased())"
            case let .scoped(id, folder): "vault:\(id.uuidString.lowercased())/\(folder.rawValue)"
            }
        }

        init?(rawValue: String) {
            if rawValue.hasPrefix("vault:") {
                let parts = rawValue.dropFirst(6).split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
                guard let id = parts.first.flatMap({ UUID(uuidString: String($0)) }) else { return nil }
                if parts.count == 1 {
                    self = .vault(id)
                } else {
                    guard let folder = Folder(rawValue: String(parts[1])), folder != .root, folder.vaultID == nil
                    else { return nil }
                    self = .scoped(id, folder)
                }
                return
            }
            switch rawValue {
            case "root": self = .root
            case "inbox": self = .inbox
            case "all": self = .all
            case "tags": self = .tags
            case "correspondents": self = .correspondents
            case "types": self = .types
            default:
                let parts = rawValue.split(separator: ":", maxSplits: 1).map(String.init)
                guard parts.count == 2, let id = UUID(uuidString: parts[1]) else { return nil }
                switch parts[0] {
                case "tag": self = .tag(id)
                case "correspondent": self = .correspondent(id)
                case "type": self = .type(id)
                default: return nil
                }
            }
        }
    }

    case folder(Folder)
    case document(UUID, in: Folder)

    public var rawValue: String {
        switch self {
        case let .folder(folder): folder.rawValue
        case let .document(id, folder): "doc:\(id.uuidString.lowercased())@\(folder.rawValue)"
        }
    }

    public init?(rawValue: String) {
        if rawValue.hasPrefix("doc:") {
            let parts = rawValue.dropFirst(4).split(separator: "@", maxSplits: 1).map(String.init)
            guard parts.count == 2, let id = UUID(uuidString: parts[0]), let folder = Folder(rawValue: parts[1]),
                folder.vaultID != nil
            else { return nil }
            self = .document(id, in: folder)
        } else if let folder = Folder(rawValue: rawValue), folder == .root || folder.vaultID != nil {
            self = .folder(folder)
        } else {
            return nil
        }
    }

    public var parent: FolioItemID? {
        switch self {
        case let .folder(folder): folder.parent.map(FolioItemID.folder)
        case let .document(_, folder): .folder(folder)
        }
    }

    /// Parent first, root last.
    public var ancestors: [FolioItemID] {
        var chain: [FolioItemID] = []
        var current = parent
        while let next = current {
            chain.append(next)
            current = next.parent
        }
        return chain
    }
}
