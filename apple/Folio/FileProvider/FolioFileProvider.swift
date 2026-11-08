import FileProvider
import FolioKit
import UniformTypeIdentifiers

/// Folio as a location in Files and Finder.
///
/// Read-only except for creating files in Inbox, which uploads them, and for what this device keeps about
/// items (Recents, tags, favorites, Keep Downloaded). Rename, move and delete are refused with a clean
/// "unsupported" error.
final class FolioFileProvider: NSObject, NSFileProviderReplicatedExtension, NSFileProviderThumbnailing,
    NSFileProviderCustomAction
{
    private let domain: NSFileProviderDomain
    private let service: (any FolioService)?
    private let temporaryDirectory: (() throws -> URL)?
    private let localMetadata: LocalItemMetadataStore

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        service = nil
        temporaryDirectory = nil
        localMetadata =
            (try? LocalItemMetadataStore.file(domain: domain)).map(LocalItemMetadataStore.shared(file:)) ?? .scratch()
        super.init()
    }

    init(
        domain: NSFileProviderDomain, service: any FolioService, temporaryDirectory: (() throws -> URL)? = nil,
        localMetadata: LocalItemMetadataStore = .scratch()
    ) {
        self.domain = domain
        self.service = service
        self.temporaryDirectory = temporaryDirectory
        self.localMetadata = localMetadata
        super.init()
    }

    private func providerItem(_ node: FolioNode) -> FolioProviderItem {
        FolioProviderItem(node, local: localMetadata.metadata(for: node.id.providerIdentifier))
    }

    func invalidate() {}

    private func catalog(downloadDirectory: URL? = nil) throws -> (FolioCatalog, any FolioService) {
        guard let service = service ?? SharedConfiguration.makeService(downloadDirectory: downloadDirectory) else {
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
                completionHandler(providerItem(node), nil)
            } catch {
                ProviderDiagnostics.failure(operation: "item", phase: "lookup", error: error)
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
            var phase = "staging"
            do {
                let directory: URL
                if let temporaryDirectory {
                    directory = try temporaryDirectory()
                } else {
                    guard let manager = NSFileProviderManager(for: domain) else {
                        throw NSFileProviderError(.providerNotFound)
                    }
                    directory = try manager.temporaryDirectoryURL()
                }
                phase = "lookup"
                let (catalog, _) = try catalog(downloadDirectory: directory)
                guard let id = FolioItemID(providerIdentifier: itemIdentifier),
                    let node = try await catalog.node(for: id), node.document != nil
                else { throw NSFileProviderError(.noSuchItem) }
                phase = "download"
                let url = try await catalog.downloadFile(for: node)
                completionHandler(url, providerItem(node), nil)
            } catch {
                ProviderDiagnostics.failure(operation: "fetchContents", phase: phase, error: error)
                completionHandler(nil, nil, error.providerError)
            }
        }
        return progress
    }

    func fetchThumbnails(
        for itemIdentifiers: [NSFileProviderItemIdentifier], requestedSize _: CGSize,
        perThumbnailCompletionHandler: @escaping (NSFileProviderItemIdentifier, Data?, (any Error)?) -> Void,
        completionHandler: @escaping ((any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: Int64(itemIdentifiers.count))
        let batch = ThumbnailBatch(
            progress: progress, perThumbnail: perThumbnailCompletionHandler, completion: completionHandler)
        let task = Task {
            var source: (any FolioService)?
            for identifier in itemIdentifiers {
                do {
                    try Task.checkCancellation()
                    guard let item = FolioItemID(providerIdentifier: identifier) else {
                        throw NSFileProviderError(.noSuchItem)
                    }
                    guard case let .document(documentID, folder) = item, let vaultID = folder.vaultID else {
                        batch.deliver(identifier, data: nil, error: nil)
                        continue
                    }
                    let service = try source ?? catalog().1
                    source = service
                    let data = try await service.scoped(to: vaultID).thumbnail(id: documentID)
                    try Task.checkCancellation()
                    batch.deliver(identifier, data: data, error: nil)
                } catch {
                    if Task.isCancelled {
                        batch.finish(CancellationError().providerError)
                        return
                    }
                    batch.deliver(identifier, data: nil, error: error.providerError)
                }
            }
            batch.finish(nil)
        }
        batch.bind(task)
        return progress
    }

    func createItem(
        basedOn itemTemplate: NSFileProviderItem, fields: NSFileProviderItemFields, contents url: URL?,
        options _: NSFileProviderCreateItemOptions = [], request _: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, (any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        guard case let .folder(destination)? = FolioItemID(providerIdentifier: itemTemplate.parentItemIdentifier),
            destination.acceptsUploads,
            itemTemplate.contentType != .folder, let url
        else {
            completionHandler(nil, [], false, Self.unsupported)
            progress.completedUnitCount = 1
            return progress
        }
        let filename = itemTemplate.filename
        Task {
            defer { progress.completedUnitCount = 1 }
            do {
                let (catalog, _) = try catalog()
                var file = try UploadFile(contentsOf: url)
                file.filename = filename
                let node = try await catalog.upload(file, to: destination)
                let local = try localMetadata.update(
                    with: itemTemplate, fields: fields, for: node.id.providerIdentifier)
                completionHandler(FolioProviderItem(node, local: local), [], false, nil)
            } catch {
                completionHandler(nil, [], false, error.providerError)
            }
        }
        return progress
    }

    /// Only the device-local fields are kept. Renames, moves and new contents are refused; other system
    /// metadata that arrives alongside is answered with the server's values, which the system then adopts.
    func modifyItem(
        _ item: NSFileProviderItem, baseVersion _: NSFileProviderItemVersion,
        changedFields: NSFileProviderItemFields, contents: URL?,
        options _: NSFileProviderModifyItemOptions = [], request _: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, (any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        let refused: NSFileProviderItemFields = [.contents, .filename, .parentItemIdentifier]
        guard contents == nil, changedFields.isDisjoint(with: refused) else {
            completionHandler(nil, [], false, Self.unsupported)
            progress.completedUnitCount = 1
            return progress
        }
        Task {
            defer { progress.completedUnitCount = 1 }
            do {
                let local = try localMetadata.update(with: item, fields: changedFields)
                let (catalog, _) = try catalog()
                guard let id = FolioItemID(providerIdentifier: item.itemIdentifier),
                    let node = try await catalog.node(for: id)
                else { throw NSFileProviderError(.noSuchItem) }
                completionHandler(FolioProviderItem(node, local: local), [], false, nil)
            } catch {
                ProviderDiagnostics.failure(operation: "modifyItem", phase: "local-metadata", error: error)
                completionHandler(nil, [], false, error.providerError)
            }
        }
        return progress
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
            return try FolioEnumerator(
                catalog: catalog, container: nil,
                directory: FolioSyncState.directory(domain: domain, container: containerItemIdentifier),
                localMetadata: localMetadata
            )
        }
        guard case let .folder(folder)? = FolioItemID(providerIdentifier: containerItemIdentifier) else {
            throw NSFileProviderError(.noSuchItem)
        }
        return try FolioEnumerator(
            catalog: catalog, container: folder,
            directory: FolioSyncState.directory(domain: domain, container: containerItemIdentifier),
            localMetadata: localMetadata
        )
    }

    /// The two Keep Downloaded menu actions; the extension's Info.plist declares them.
    func performAction(
        identifier actionIdentifier: NSFileProviderExtensionActionIdentifier,
        onItemsWithIdentifiers itemIdentifiers: [NSFileProviderItemIdentifier],
        completionHandler: @escaping ((any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        defer { progress.completedUnitCount = 1 }
        let keep: Bool
        switch actionIdentifier {
        case FolioProviderItem.keepDownloadedAction: keep = true
        case FolioProviderItem.allowEvictionAction: keep = false
        default:
            completionHandler(Self.unsupported)
            return progress
        }
        do {
            try localMetadata.setKeepDownloaded(keep, for: itemIdentifiers.filter { $0 != .rootContainer })
        } catch {
            ProviderDiagnostics.failure(operation: "performAction", phase: "local-metadata", error: error)
            completionHandler(error.providerError)
            return progress
        }
        completionHandler(nil)
        NSFileProviderManager(for: domain)?.signalEnumerator(for: .workingSet) { _ in }
        return progress
    }

    private static var unsupported: NSError {
        NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError)
    }
}

final class FolioEnumerator: NSObject, NSFileProviderEnumerator {
    private let state: FolioSyncState
    private let localMetadata: LocalItemMetadataStore

    init(
        catalog: FolioCatalog, container: FolioItemID.Folder?, directory: URL,
        localMetadata: LocalItemMetadataStore = .scratch()
    ) {
        state = FolioSyncState(
            catalog: catalog, container: container, directory: directory, localMetadata: localMetadata)
        self.localMetadata = localMetadata
    }

    private func providerItem(_ node: FolioNode) -> FolioProviderItem {
        FolioProviderItem(node, local: localMetadata.metadata(for: node.id.providerIdentifier))
    }

    func invalidate() {}

    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt _: NSFileProviderPage) {
        Task {
            do {
                let snapshot = try await state.initialSnapshot()
                observer.didEnumerate(snapshot.nodes.map(providerItem))
                observer.finishEnumerating(upTo: nil)
            } catch {
                observer.finishEnumeratingWithError(error.providerError)
            }
        }
    }

    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        Task {
            do {
                let (snapshot, updated, deleted) = try await state.changes(from: anchor)
                observer.didUpdate(updated.map(providerItem))
                observer.didDeleteItems(withIdentifiers: deleted)
                observer.finishEnumeratingChanges(upTo: snapshot.anchor, moreComing: false)
            } catch {
                observer.finishEnumeratingWithError(error.providerError)
            }
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        Task {
            completionHandler(try? await state.initialSnapshot().anchor)
        }
    }
}

final class FolioProviderItem: NSObject, NSFileProviderItem {
    private let node: FolioNode
    private let local: LocalItemMetadata

    init(_ node: FolioNode, local: LocalItemMetadata) {
        self.node = node
        self.local = local
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
            return node.canUpload
                ? [.allowsContentEnumerating, .allowsAddingSubItems] : [.allowsContentEnumerating]
        }
        return [.allowsReading]
    }

    var contentModificationDate: Date? {
        node.isFolder ? node.created : node.modified
    }

    var creationDate: Date? {
        node.created
    }

    var lastUsedDate: Date? {
        local.lastUsedDate
    }

    var tagData: Data? {
        local.tagData
    }

    #if !os(macOS)
        var favoriteRank: NSNumber? {
            local.favoriteRank.map { NSNumber(value: $0) }
        }
    #endif

    static let keepDownloadedAction = NSFileProviderExtensionActionIdentifier("com.cedricziel.folio.keep-downloaded")
    static let allowEvictionAction = NSFileProviderExtensionActionIdentifier("com.cedricziel.folio.allow-eviction")

    /// Read by the menu actions' activation rules. Only macOS can keep an item downloaded, so only macOS
    /// offers the actions.
    var userInfo: [AnyHashable: Any]? {
        #if os(macOS)
            return ["canKeepDownloaded": true, "keptDownloaded": local.keepDownloaded == true]
        #else
            return [:]
        #endif
    }

    #if os(macOS)
        /// Contents of a kept folder report `.inherited` and so stay downloaded too.
        var contentPolicy: NSFileProviderContentPolicy {
            local.keepDownloaded == true ? .downloadEagerlyAndKeepDownloaded : .inherited
        }
    #endif

    var itemVersion: NSFileProviderItemVersion {
        let stamp = Data(String(node.modified.timeIntervalSince1970).utf8)
        return NSFileProviderItemVersion(
            contentVersion: stamp, metadataVersion: FolioSyncState.metadataVersion(node, local: local))
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

private final class ThumbnailBatch {
    private let queue = DispatchQueue(label: "com.cedricziel.folio.thumbnail-callbacks")
    private let progress: Progress
    private let perThumbnail: (NSFileProviderItemIdentifier, Data?, (any Error)?) -> Void
    private let completion: ((any Error)?) -> Void
    private var finished = false

    init(
        progress: Progress,
        perThumbnail: @escaping (NSFileProviderItemIdentifier, Data?, (any Error)?) -> Void,
        completion: @escaping ((any Error)?) -> Void
    ) {
        self.progress = progress
        self.perThumbnail = perThumbnail
        self.completion = completion
    }

    func bind(_ task: Task<Void, Never>) {
        queue.async {
            guard !self.finished else { return }
            self.progress.cancellationHandler = {
                task.cancel()
                self.finish(CancellationError().providerError)
            }
            if self.progress.isCancelled {
                task.cancel()
                self.finish(CancellationError().providerError)
            }
        }
    }

    func deliver(_ identifier: NSFileProviderItemIdentifier, data: Data?, error: (any Error)?) {
        queue.async {
            guard !self.finished, !self.progress.isCancelled else { return }
            self.progress.completedUnitCount += 1
            self.perThumbnail(identifier, data, error)
        }
    }

    func finish(_ error: (any Error)?) {
        queue.async {
            guard !self.finished else { return }
            self.finished = true
            self.progress.cancellationHandler = nil
            self.completion(self.progress.isCancelled ? CancellationError().providerError : error)
        }
    }
}
