import FileProvider
import FolioKit
import UniformTypeIdentifiers

/// Folio as a location in Files and Finder.
///
/// Read-only except for creating files in Inbox, which uploads them. Rename, move and delete are refused
/// with a clean "unsupported" error. Thumbnails are not provided: the API has no thumbnail endpoint yet.
final class FolioFileProvider: NSObject, NSFileProviderReplicatedExtension {
    private let domain: NSFileProviderDomain

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        super.init()
    }

    func invalidate() {}

    private func catalog(downloadDirectory: URL? = nil) throws -> (FolioCatalog, any FolioService) {
        guard let service = SharedConfiguration.makeService(downloadDirectory: downloadDirectory) else {
            throw NSFileProviderError(.notAuthenticated)
        }
        return (FolioCatalog(service: service), service)
    }

    func item(
        for identifier: NSFileProviderItemIdentifier, request _: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, (any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        Task {
            defer { progress.completedUnitCount = 1 }
            do {
                let (catalog, _) = try catalog()
                guard let id = FolioItemID(providerIdentifier: identifier),
                    let node = try await catalog.node(for: id)
                else { return completionHandler(nil, NSFileProviderError(.noSuchItem)) }
                completionHandler(FolioProviderItem(node), nil)
            } catch {
                completionHandler(nil, error.providerError)
            }
        }
        return progress
    }

    func fetchContents(
        for itemIdentifier: NSFileProviderItemIdentifier, version _: NSFileProviderItemVersion?,
        request _: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, (any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        Task {
            defer { progress.completedUnitCount = 1 }
            do {
                guard let manager = NSFileProviderManager(for: domain) else {
                    throw NSFileProviderError(.providerNotFound)
                }
                let (catalog, service) = try catalog(downloadDirectory: manager.temporaryDirectoryURL())
                guard let id = FolioItemID(providerIdentifier: itemIdentifier),
                    let node = try await catalog.node(for: id), let document = node.document
                else { return completionHandler(nil, nil, NSFileProviderError(.noSuchItem)) }
                let url = try await service.downloadFile(for: document)
                completionHandler(url, FolioProviderItem(node), nil)
            } catch {
                completionHandler(nil, nil, error.providerError)
            }
        }
        return progress
    }

    func createItem(
        basedOn itemTemplate: NSFileProviderItem, fields _: NSFileProviderItemFields, contents url: URL?,
        options _: NSFileProviderCreateItemOptions = [], request _: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, (any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        guard itemTemplate.parentItemIdentifier == FolioItemID.folder(.inbox).providerIdentifier,
            itemTemplate.contentType != .folder, let url
        else {
            completionHandler(nil, [], false, Self.unsupported)
            progress.completedUnitCount = 1
            return progress
        }
        Task {
            defer { progress.completedUnitCount = 1 }
            do {
                let (catalog, service) = try catalog()
                var file = try UploadFile(contentsOf: url)
                file.filename = itemTemplate.filename
                let document = try await service.upload(file)
                let id = FolioItemID.document(document.id, in: .inbox)
                let node = try await catalog.node(for: id)
                completionHandler(node.map(FolioProviderItem.init), [], false, nil)
            } catch {
                completionHandler(nil, [], false, error.providerError)
            }
        }
        return progress
    }

    func modifyItem(
        _: NSFileProviderItem, baseVersion _: NSFileProviderItemVersion,
        changedFields _: NSFileProviderItemFields, contents _: URL?,
        options _: NSFileProviderModifyItemOptions = [], request _: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, (any Error)?) -> Void
    ) -> Progress {
        completionHandler(nil, [], false, Self.unsupported)
        return Progress()
    }

    func deleteItem(
        identifier _: NSFileProviderItemIdentifier, baseVersion _: NSFileProviderItemVersion,
        options _: NSFileProviderDeleteItemOptions = [], request _: NSFileProviderRequest,
        completionHandler: @escaping ((any Error)?) -> Void
    ) -> Progress {
        completionHandler(Self.unsupported)
        return Progress()
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier, request _: NSFileProviderRequest
    ) throws -> any NSFileProviderEnumerator {
        let (catalog, _) = try catalog()
        if containerItemIdentifier == .workingSet {
            return FolioEnumerator(catalog: catalog, container: nil)
        }
        guard case let .folder(folder)? = FolioItemID(providerIdentifier: containerItemIdentifier) else {
            throw NSFileProviderError(.noSuchItem)
        }
        return FolioEnumerator(catalog: catalog, container: folder)
    }

    private static var unsupported: NSError {
        NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError)
    }
}

/// Lists a folder, or the whole archive for the working set. The working set polls the document list and
/// compares a change token; once the server pushes `/events`, `signalEnumerator` can replace the polling.
final class FolioEnumerator: NSObject, NSFileProviderEnumerator {
    private let catalog: FolioCatalog
    private let container: FolioItemID.Folder?

    init(catalog: FolioCatalog, container: FolioItemID.Folder?) {
        self.catalog = catalog
        self.container = container
    }

    func invalidate() {}

    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt _: NSFileProviderPage) {
        Task {
            do {
                let nodes: [FolioNode]
                if let container {
                    nodes = try await catalog.children(of: container)
                } else {
                    nodes = try await catalog.allNodes()
                }
                observer.didEnumerate(nodes.map(FolioProviderItem.init))
                observer.finishEnumerating(upTo: nil)
            } catch {
                observer.finishEnumeratingWithError(error.providerError)
            }
        }
    }

    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        Task {
            do {
                let token = try await catalog.changeToken()
                let newAnchor = NSFileProviderSyncAnchor(Data(token.utf8))
                if container == nil, anchor.rawValue != newAnchor.rawValue {
                    try observer.didUpdate(await catalog.allNodes().map(FolioProviderItem.init))
                }
                observer.finishEnumeratingChanges(upTo: newAnchor, moreComing: false)
            } catch {
                observer.finishEnumeratingWithError(error.providerError)
            }
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        Task {
            completionHandler((try? await catalog.changeToken()).map { NSFileProviderSyncAnchor(Data($0.utf8)) })
        }
    }
}

final class FolioProviderItem: NSObject, NSFileProviderItem {
    private let node: FolioNode

    init(_ node: FolioNode) {
        self.node = node
    }

    var itemIdentifier: NSFileProviderItemIdentifier {
        node.id.providerIdentifier
    }

    var parentItemIdentifier: NSFileProviderItemIdentifier {
        node.parent.providerIdentifier
    }

    var filename: String {
        node.name
    }

    var contentType: UTType {
        node.isFolder ? .folder : (node.contentType.flatMap { UTType(mimeType: $0) } ?? .data)
    }

    var capabilities: NSFileProviderItemCapabilities {
        if node.isFolder {
            return node.id == .folder(.inbox)
                ? [.allowsContentEnumerating, .allowsAddingSubItems] : [.allowsContentEnumerating]
        }
        return [.allowsReading]
    }

    var contentModificationDate: Date? {
        node.isFolder ? nil : node.modified
    }

    var creationDate: Date? {
        node.isFolder ? nil : node.document?.createdAt
    }

    var itemVersion: NSFileProviderItemVersion {
        let stamp = Data(String(node.modified.timeIntervalSince1970).utf8)
        return NSFileProviderItemVersion(contentVersion: stamp, metadataVersion: stamp)
    }
}

extension FolioItemID {
    var providerIdentifier: NSFileProviderItemIdentifier {
        self == .folder(.root) ? .rootContainer : NSFileProviderItemIdentifier(rawValue)
    }

    init?(providerIdentifier: NSFileProviderItemIdentifier) {
        if providerIdentifier == .rootContainer {
            self = .folder(.root)
        } else {
            self.init(rawValue: providerIdentifier.rawValue)
        }
    }
}

extension Error {
    /// Files shows its sign-in prompt for `.notAuthenticated`.
    var providerError: any Error {
        (self as? FolioError) == .signInRequired ? NSFileProviderError(.notAuthenticated) : self
    }
}
