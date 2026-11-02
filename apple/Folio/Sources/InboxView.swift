import CoreSpotlight
import FolioKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct InboxView: View {
    let serverURL: URL
    let service: any FolioService
    let vaultID: UUID?
    let canEdit: Bool
    let canUpload: Bool
    let makeUploadDestination: () -> VaultDestinationViewModel
    let onSignOut: () -> Void

    @Bindable var session: DocumentBrowserSession
    let onNavigate: (WorkspaceDestination) -> Void
    let onOpenDocument: (Document.ID, UUID?) -> Void

    private var list: DocumentListViewModel {
        session.list
    }

    @State private var uploads: UploadViewModel
    @State private var showFileImporter = false
    @State private var photoSelection: [PhotosPickerItem] = []
    @State private var pendingDelete: Document?
    @State private var showScanner = false
    @State private var problem: String?
    @State private var scannedPages: ScannedPages?
    @State private var pendingUploads: UploadBatch?
    @State private var pageTask: Task<Void, Never>?
    @FocusState private var searchFocused: Bool
    private let router = IntentRouter.shared

    init(
        serverURL: URL, service: any FolioService, session: DocumentBrowserSession,
        vaultID: UUID?,
        canEdit: Bool, canUpload: Bool,
        makeUploadDestination: @escaping () -> VaultDestinationViewModel,
        onSignOut: @escaping () -> Void,
        onNavigate: @escaping (WorkspaceDestination) -> Void,
        onOpenDocument: @escaping (Document.ID, UUID?) -> Void
    ) {
        self.serverURL = serverURL
        self.service = service
        self.vaultID = vaultID
        self.canEdit = canEdit
        self.canUpload = canUpload
        self.makeUploadDestination = makeUploadDestination
        self.session = session
        self.onSignOut = onSignOut
        self.onNavigate = onNavigate
        self.onOpenDocument = onOpenDocument
        let list = session.list
        _uploads = State(
            initialValue: UploadViewModel(service: service) {
                Task { await list.load() }
            }
        )
    }

    var body: some View {
        NavigationStack(path: $session.path) {
            sidebar
                .navigationDestination(for: Document.ID.self) { id in
                    documentDestination(id)
                }
        }
        .onChange(of: session.path) { _, path in
            if let id = path.last {
                session.selection = id
            }
        }
        // Keep loading alive when compact navigation hides the sidebar.
        .task(id: list.query) { await list.search(list.searchText) }
        .onChange(of: list.query) { pageTask?.cancel() }
        .onDisappear { pageTask?.cancel() }
        .focusedSceneValue(\.inboxActions, actions)
        .onChange(of: router.request, initial: true) { _, request in
            guard let request else { return }
            router.clear()
            handle(request)
        }
        .onContinueUserActivity(CSSearchableItemActionType) { activity in
            let identifier = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String
            if let link = identifier.flatMap(DocumentLink.init(spotlightIdentifier:)) {
                handle(.document(link.documentID, vaultID: link.vaultID))
            }
        }
    }

    private func documentDestination(_ id: Document.ID) -> some View {
        Group {
            if let document = list.document(for: id) {
                DocumentDetailView(
                    model: DocumentDetailViewModel(
                        document: document, correspondent: list.correspondentName(for: document),
                        type: list.typeName(for: document), service: service
                    ), vaultID: vaultID
                )
                .id(id)
            } else if let message = list.documentError(for: id) {
                ContentUnavailableView {
                    Label("Document unavailable", systemImage: "doc.text")
                } description: {
                    Text(message)
                } actions: {
                    Button("Try again") { Task { await list.resolveDocument(id) } }
                }
            } else {
                ProgressView("Loading document…")
            }
        }
        .task(id: id) { await list.resolveDocument(id) }
    }

    private var actions: InboxActions {
        InboxActions(
            upload: canUpload ? { showFileImporter = true } : nil,
            scan: scanAction,
            showInbox: { onNavigate(.inbox) },
            showAll: { onNavigate(.all) },
            find: { searchFocused = true },
            deleteSelected: canEdit
                ? session.selection.flatMap { list.document(for: $0) }.map { document in
                    { pendingDelete = document }
                } : nil
        )
    }

    private var scanAction: (() -> Void)? {
        guard canUpload else { return nil }
        #if os(iOS)
            return DocumentScanner.isSupported ? { showScanner = true } : nil
        #else
            return nil
        #endif
    }

    private func handle(_ request: IntentRouter.Request) {
        switch request {
        case .scan:
            guard canUpload else { return }
            if let scan = scanAction {
                scan()
            } else {
                // Macs and simulators have no scanner, so scanning falls back to choosing a file.
                showFileImporter = true
            }
        case .upload:
            guard canUpload else { return }
            showFileImporter = true
        case .inbox:
            onNavigate(.inbox)
        case let .document(id, vaultID):
            onOpenDocument(id, vaultID)
        }
    }

    private var sidebar: some View {
        @Bindable var list = session.list
        return List(selection: $session.selection) {
            ForEach(list.rows) { row in
                NavigationLink(value: row.id) {
                    DocumentRowView(row: row)
                }
                .tag(row.id)
                .listRowBackground(row.id == session.selection ? Color.archiveTint : Color.paper)
                .listRowSeparatorTint(.rule)
                .swipeActions {
                    if canEdit {
                        Button("Delete", systemImage: "trash", role: .destructive) {
                            pendingDelete = row.document
                        }
                    }
                }
            }
            if list.phase == .loading && !list.rows.isEmpty {
                ProgressView("Loading documents…")
            } else if list.canLoadMore {
                VStack(spacing: 8) {
                    if let message = list.pageError {
                        Text(message).foregroundStyle(Color.danger)
                    }
                    if list.isLoadingMore {
                        ProgressView("Loading documents…")
                    } else {
                        Button(list.pageError == nil ? "Load more documents" : "Retry loading documents") {
                            pageTask?.cancel()
                            pageTask = Task { await list.loadNextPage() }
                        }
                        .buttonStyle(.borderless)
                    }
                }
                .frame(maxWidth: .infinity)
                .listRowBackground(Color.paper)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.paper)
        .navigationTitle(list.scope == .inbox ? "Inbox" : "All documents")
        .searchable(text: $list.searchText, prompt: "Search documents")
        .searchFocused($searchFocused)
        .refreshable { await list.load() }
        .overlay { placeholder }
        .safeAreaInset(edge: .top, spacing: 0) { uploadNotice }
        .toolbar { toolbar }
        #if os(iOS)
            .safeAreaInset(edge: .bottom) { scanButton }
            .fullScreenCover(isPresented: $showScanner) {
                DocumentScanner(
                    onFinish: { pages in
                        showScanner = false
                        scannedPages = ScannedPages(images: pages)
                    },
                    onCancel: { showScanner = false }
                )
            }
        #endif
        .sheet(item: $scannedPages) { scan in
            ScanReviewView(
                model: ScanReviewViewModel(
                    pages: scan.images, service: service, destination: makeUploadDestination()
                ),
                onFinished: {
                    scannedPages = nil
                    Task { await list.load() }
                }
            )
        }
        .sheet(item: $pendingUploads) { batch in
            UploadReviewView(files: batch.files, model: uploads) { pendingUploads = nil }
        }
        .fileImporter(
            isPresented: $showFileImporter, allowedContentTypes: [.pdf, .image], allowsMultipleSelection: true
        ) { result in
            switch result {
            case let .failure(error):
                problem = error.localizedDescription
            case let .success(urls):
                Task {
                    let loaded = await Self.readFiles(urls)
                    if !loaded.failed.isEmpty {
                        problem = "Couldn't read \(loaded.failed.joined(separator: ", "))."
                    }
                    reviewUploads(loaded.files)
                }
            }
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(
                get: { problem != nil || list.actionError != nil },
                set: {
                    if !$0 {
                        problem = nil; list.dismissActionError()
                    }
                }
            )
        ) {
            Button("OK") {}
        } message: {
            Text(problem ?? list.actionError ?? "")
        }
        .onChange(of: photoSelection) { _, items in
            guard !items.isEmpty else { return }
            photoSelection = []
            Task { reviewUploads(await loadPhotos(items)) }
        }
        .confirmationDialog(
            "Delete this document?",
            isPresented: Binding(
                get: { pendingDelete != nil },
                set: {
                    if !$0 {
                        pendingDelete = nil
                    }
                }
            ),
            presenting: pendingDelete
        ) { document in
            Button("Delete", role: .destructive) {
                Task { await list.delete(document) }
            }
            Button("Keep", role: .cancel) {}
        } message: { document in
            Text("\(document.title) will be removed from your archive.")
        }
    }

    @ViewBuilder
    private var placeholder: some View {
        switch list.phase {
        case .loading where list.rows.isEmpty,
            .idle where list.rows.isEmpty:
            Text("Reading your inbox…")
                .font(FolioTheme.Typography.body)
                .foregroundStyle(Color.inkMuted)
        case let .failed(message):
            ContentUnavailableView {
                Label("Couldn't load documents", systemImage: "exclamationmark.triangle")
                    .foregroundStyle(Color.danger)
            } description: {
                Text(message)
            } actions: {
                Button("Try again") { Task { await list.load() } }
                Button("Sign out", action: onSignOut)
            }
        case .loaded where list.rows.isEmpty:
            ContentUnavailableView {
                Label(
                    list.searchText.isEmpty ? "Nothing here yet" : "No matches",
                    systemImage: "tray.and.arrow.down"
                )
            } description: {
                Text(
                    list.searchText.isEmpty
                        ? "Scan a letter or choose a file to file your first document."
                        : "Try a different word."
                )
            }
        default:
            EmptyView()
        }
    }

    @ViewBuilder
    private var uploadNotice: some View {
        switch uploads.state {
        case .idle:
            EmptyView()
        case let .uploading(done, total):
            notice("Filing \(done + 1) of \(total)…", icon: "tray.and.arrow.down", fill: .pendingTint, text: .pending)
        case let .finished(count):
            notice(
                "Filed \(count) \(count == 1 ? "document" : "documents").", icon: "checkmark", fill: .archiveTint,
                text: .ink
            )
            .task {
                guard (try? await Task.sleep(for: .seconds(3))) != nil else { return }
                uploads.dismiss()
            }
        case let .failed(message, _):
            notice(message, icon: "exclamationmark.triangle", fill: .dangerTint, text: .danger)
                .onTapGesture { uploads.dismiss() }
        }
    }

    private func notice(_ message: String, icon: String, fill: Color, text: Color) -> some View {
        Label(message, systemImage: icon)
            .font(FolioTheme.Typography.bodySm)
            .foregroundStyle(text)
            .padding(FolioTheme.Spacing.space3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(fill)
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            Button("Choose files", systemImage: "tray.and.arrow.down") { showFileImporter = true }
                .help("Upload files (\u{2318}O)")
                .disabled(!canUpload)
            PhotosPicker(selection: $photoSelection, matching: .images) {
                Label("Photos", systemImage: "photo")
            }
            .disabled(!canUpload)
            Menu("Server", systemImage: "gearshape") {
                Text(serverURL.absoluteString)
                Button("Sign out", action: onSignOut)
            }
        }
    }

    #if os(iOS)
        private var scanButton: some View {
            Button {
                showScanner = true
            } label: {
                Label("Scan", systemImage: "doc.viewfinder").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(FolioTheme.Spacing.space4)
            .disabled(!canUpload || !DocumentScanner.isSupported)
        }
    #endif

    private func reviewUploads(_ files: [UploadFile]) {
        guard !files.isEmpty else { return }
        let list = session.list
        uploads = UploadViewModel(service: service, destination: makeUploadDestination()) {
            Task { await list.load() }
        }
        pendingUploads = UploadBatch(files: files)
    }

    /// Reads off the main actor so large files do not freeze the UI.
    private nonisolated static func readFiles(_ urls: [URL]) async -> (files: [UploadFile], failed: [String]) {
        await Task.detached {
            var files: [UploadFile] = []
            var failed: [String] = []
            for url in urls {
                if let file = try? UploadFile(contentsOf: url) {
                    files.append(file)
                } else {
                    failed.append(url.lastPathComponent)
                }
            }
            return (files, failed)
        }.value
    }

    private func loadPhotos(_ items: [PhotosPickerItem]) async -> [UploadFile] {
        var files: [UploadFile] = []
        for item in items {
            guard let data = try? await item.loadTransferable(type: Data.self) else { continue }
            let type = item.supportedContentTypes.first ?? .jpeg
            let name = "Photo \(FolioFormat.isoDay(Date())) \(files.count + 1)"
            files.append(
                UploadFile(
                    data: data, filename: "\(name).\(type.preferredFilenameExtension ?? "jpg")",
                    mimeType: type.preferredMIMEType ?? "image/jpeg", title: name
                )
            )
        }
        return files
    }
}

private struct UploadBatch: Identifiable {
    let id = UUID()
    let files: [UploadFile]
}

struct ScannedPages: Identifiable {
    let id = UUID()
    let images: [CGImage]
}
