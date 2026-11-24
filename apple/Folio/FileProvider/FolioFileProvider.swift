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
    private let temporaryDirectory: (() throws -> URL)?
    private let context: ProviderContext

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        temporaryDirectory = nil
        let localMetadata =
            (try? LocalItemMetadataStore.file(domain: domain)).map(LocalItemMetadataStore.shared(file:)) ?? .scratch()
        context = ProviderContext(localMetadata: localMetadata, service: nil, cache: nil)
        super.init()
    }

    init(
        domain: NSFileProviderDomain, service: any FolioService, temporaryDirectory: (() throws -> URL)? = nil,
        localMetadata: LocalItemMetadataStore = .scratch(), cache: FolioCatalogCache? = nil
    ) {
        self.domain = domain
        self.temporaryDirectory = temporaryDirectory
        context = ProviderContext(localMetadata: localMetadata, service: service, cache: cache)
        super.init()
    }

    func invalidate() {}

    private func downloadStagingDirectory() throws -> URL {
        if let temporaryDirectory {
            return try temporaryDirectory()
        }
        guard let manager = NSFileProviderManager(for: domain) else {
            throw NSFileProviderError(.providerNotFound)
        }
        return try manager.temporaryDirectoryURL()
    }

    func item(
        for identifier: NSFileProviderItemIdentifier, request _: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, (any Error)?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        let reply = FileProviderCallback(completionHandler)
        Task { [context] in
            defer {
                progress.completedUnitCount = 1
                FolioTelemetry.shared.flush()
            }
            do {
                let (catalog, _) = try context.catalog()
                guard let id = FolioItemID(providerIdentifier: identifier),
                    let node = try await catalog.node(for: id)
                else { return reply(nil, NSFileProviderError(.noSuchItem)) }
                reply(context.providerItem(node), nil)
            } catch {
                ProviderDiagnostics.failure(operation: "item", phase: "lookup", error: error)
                reply(nil, error.providerError)
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
        let stagingDirectory = Result { try downloadStagingDirectory() }
        let reply = FileProviderCallback(completionHandler)
        let task = Task { [context] in
            defer {
                progress.completedUnitCount = 1
                FolioTelemetry.shared.flush()
            }
            var phase = "staging"
            do {
                try Task.checkCancellation()
                let directory = try stagingDirectory.get()
                phase = "lookup"
                let (catalog, _) = try context.catalog(downloadDirectory: directory)
                guard let id = FolioItemID(providerIdentifier: itemIdentifier),
                    let node = try await catalog.node(for: id), node.document != nil
                else { throw NSFileProviderError(.noSuchItem) }
                try Task.checkCancellation()
                phase = "download"
                let url = try await catalog.downloadFile(for: node)
                if progress.isCancelled || Task.isCancelled {
                    try? FileManager.default.removeItem(at: url)
                    throw CancellationError()
                }
                reply(url, context.providerItem(node), nil)
            } catch {
                let failure = progress.isCancelled || Task.isCancelled ? CancellationError() : error
                ProviderDiagnostics.failure(operation: "fetchContents", phase: phase, error: failure)
                reply(nil, nil, failure.providerError)
            }
        }
        progress.cancellationHandler = { task.cancel() }
        if progress.isCancelled {
            task.cancel()
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
            progress: progress, perThumbnail: FileProviderCallback(perThumbnailCompletionHandler),
            completion: FileProviderCallback(completionHandler)
        )
        let task = Task { [context] in
            defer { FolioTelemetry.shared.flush() }
            var source: (any FolioService)?
            for identifier in itemIdentifiers {
                do {
                    try Task.checkCancellation()
                    guard let item = FolioItemID(providerIdentifier: identifier) else {
                        throw NSFileProviderError(.noSuchItem)
                    }
                    guard case let .document(documentID, folder) = item, let vaultID = folder.vaultID else {
                        await batch.deliver(identifier, data: nil, error: nil)
                        continue
                    }
                    let service = try source ?? context.catalog().1
                    source = service
                    let data = try await service.scoped(to: vaultID).thumbnail(id: documentID)
                    try Task.checkCancellation()
                    await batch.deliver(identifier, data: data, error: nil)
                } catch {
                    if Task.isCancelled {
                        await batch.finish(CancellationError().providerError)
                        return
                    }
                    await batch.deliver(identifier, data: nil, error: error.providerError)
                }
            }
            await batch.finish(nil)
        }
        Task { await batch.bind(task) }
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
        let edit = LocalItemMetadataEdit(item: itemTemplate, fields: fields)
        let reply = FileProviderCallback(completionHandler)
        Task { [context] in
            defer {
                progress.completedUnitCount = 1
                FolioTelemetry.shared.flush()
            }
            do {
                let (catalog, _) = try context.catalog()
                var file = try UploadFile(contentsOf: url)
                file.filename = filename
                let node = try await catalog.upload(file, to: destination)
                // The upload already happened, so failing here would make the system upload it again
                let local: LocalItemMetadata
                do {
                    local = try context.localMetadata.update(with: edit, for: node.id.providerIdentifier)
                } catch {
                    ProviderDiagnostics.failure(operation: "createItem", phase: "local-metadata", error: error)
                    local = context.localMetadata.metadata(for: node.id.providerIdentifier)
                }
                reply(FolioProviderItem(node, local: local), [], false, nil)
            } catch {
                reply(nil, [], false, error.providerError)
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
        let identifier = item.itemIdentifier
        let edit = LocalItemMetadataEdit(item: item, fields: changedFields)
        let reply = FileProviderCallback(completionHandler)
        Task { [context] in
            defer {
                progress.completedUnitCount = 1
                FolioTelemetry.shared.flush()
            }
            do {
                let local = try context.localMetadata.update(with: edit, for: identifier)
                let (catalog, _) = try context.catalog()
                guard let id = FolioItemID(providerIdentifier: identifier),
                    let node = try await catalog.node(for: id)
                else { throw NSFileProviderError(.noSuchItem) }
                reply(FolioProviderItem(node, local: local), [], false, nil)
            } catch {
                ProviderDiagnostics.failure(operation: "modifyItem", phase: "local-metadata", error: error)
                reply(nil, [], false, error.providerError)
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
        let (catalog, _) = try context.catalog()
        if containerItemIdentifier == .workingSet {
            return try FolioEnumerator(
                catalog: catalog, container: nil,
                directory: FolioSyncState.directory(domain: domain, container: containerItemIdentifier),
                localMetadata: context.localMetadata
            )
        }
        guard case let .folder(folder)? = FolioItemID(providerIdentifier: containerItemIdentifier) else {
            throw NSFileProviderError(.noSuchItem)
        }
        return try FolioEnumerator(
            catalog: catalog, container: folder,
            directory: FolioSyncState.directory(domain: domain, container: containerItemIdentifier),
            localMetadata: context.localMetadata
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
            try context.localMetadata.setKeepDownloaded(keep, for: itemIdentifiers.filter { $0 != .rootContainer })
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

private struct ProviderContext: Sendable {
    let localMetadata: LocalItemMetadataStore
    let service: (any FolioService)?
    let cache: FolioCatalogCache?
    private static let catalogCaches = FolioCatalogCaches()

    func providerItem(_ node: FolioNode) -> FolioProviderItem {
        FolioProviderItem(node, local: localMetadata.metadata(for: node.id.providerIdentifier))
    }

    func catalog(downloadDirectory: URL? = nil) throws -> (FolioCatalog, any FolioService) {
        if let service {
            return (FolioCatalog(service: service, cache: cache), service)
        }
        guard let server = SharedConfiguration.serverURL() else {
            Self.catalogCaches.clear()
            throw NSFileProviderError(.notAuthenticated)
        }
        // One read of the server keeps the service and the cache it fills on the same server.
        let service = SharedConfiguration.makeService(for: server, downloadDirectory: downloadDirectory)
        let credentials: OAuthCredentials?
        do {
            credentials = try KeychainCredentialStore().load()
        } catch {
            // Lookups still work, uncached; requests report the keychain error themselves.
            ProviderDiagnostics.failure(operation: "catalog", phase: "credentials", error: error)
            credentials = nil
        }
        let cache = Self.catalogCaches.cache(for: server, credentials: credentials)
        return (FolioCatalog(service: service, cache: cache), service)
    }
}

final class FolioEnumerator: NSObject, NSFileProviderEnumerator, Sendable {
    private let state: FolioSyncState
    private let localMetadata: LocalItemMetadataStore

    init(
        catalog: FolioCatalog, container: FolioItemID.Folder?, directory: URL,
        localMetadata: LocalItemMetadataStore = .scratch()
    ) {
        state = FolioSyncState(
            catalog: catalog, container: container, directory: directory, localMetadata: localMetadata
        )
        self.localMetadata = localMetadata
    }

    private func providerItem(_ node: FolioNode) -> FolioProviderItem {
        FolioProviderItem(node, local: localMetadata.metadata(for: node.id.providerIdentifier))
    }

    func invalidate() {}

    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt _: NSFileProviderPage) {
        let notify = FileProviderCallback { (notification: @escaping (NSFileProviderEnumerationObserver) -> Void) in
            notification(observer)
        }
        Task {
            defer { FolioTelemetry.shared.flush() }
            do {
                let snapshot = try await state.initialSnapshot()
                let items = snapshot.nodes.map(providerItem)
                notify { observer in
                    observer.didEnumerate(items)
                    observer.finishEnumerating(upTo: nil)
                }
            } catch {
                notify { $0.finishEnumeratingWithError(error.providerError) }
            }
        }
    }

    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        let notify = FileProviderCallback { (notification: @escaping (NSFileProviderChangeObserver) -> Void) in
            notification(observer)
        }
        Task {
            defer { FolioTelemetry.shared.flush() }
            do {
                let (snapshot, updated, deleted) = try await state.changes(from: anchor)
                let items = updated.map(providerItem)
                notify { observer in
                    observer.didUpdate(items)
                    observer.didDeleteItems(withIdentifiers: deleted)
                    observer.finishEnumeratingChanges(upTo: snapshot.anchor, moreComing: false)
                }
            } catch {
                notify { $0.finishEnumeratingWithError(error.providerError) }
            }
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        let reply = FileProviderCallback(completionHandler)
        Task {
            reply(try? await state.initialSnapshot().anchor)
        }
    }
}

final class FolioProviderItem: NSObject, NSFileProviderItem, Sendable {
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

    var documentSize: NSNumber? {
        node.document?.fileSize.map { NSNumber(value: $0) }
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
            contentVersion: stamp, metadataVersion: FolioSyncState.metadataVersion(node, local: local)
        )
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

/// File Provider supplies these blocks and observers for asynchronous replies, but the SDK omits Sendable.
private struct FileProviderCallback<each Argument>: @unchecked Sendable {
    private let callback: (repeat each Argument) -> Void

    init(_ callback: @escaping (repeat each Argument) -> Void) {
        self.callback = callback
    }

    func callAsFunction(_ arguments: repeat each Argument) {
        callback(repeat each arguments)
    }
}

private actor ThumbnailBatch {
    private let progress: Progress
    private let perThumbnail: FileProviderCallback<NSFileProviderItemIdentifier, Data?, (any Error)?>
    private let completion: FileProviderCallback<(any Error)?>
    private var finished = false

    init(
        progress: Progress,
        perThumbnail: FileProviderCallback<NSFileProviderItemIdentifier, Data?, (any Error)?>,
        completion: FileProviderCallback<(any Error)?>
    ) {
        self.progress = progress
        self.perThumbnail = perThumbnail
        self.completion = completion
    }

    func bind(_ task: Task<Void, Never>) {
        guard !finished else { return }
        progress.cancellationHandler = {
            task.cancel()
            Task { await self.finish(CancellationError().providerError) }
        }
        if progress.isCancelled {
            task.cancel()
            finish(CancellationError().providerError)
        }
    }

    func deliver(_ identifier: NSFileProviderItemIdentifier, data: Data?, error: (any Error)?) {
        guard !finished, !progress.isCancelled else { return }
        progress.completedUnitCount += 1
        perThumbnail(identifier, data, error)
    }

    func finish(_ error: (any Error)?) {
        guard !finished else { return }
        finished = true
        progress.cancellationHandler = nil
        completion(progress.isCancelled ? CancellationError().providerError : error)
    }
}
