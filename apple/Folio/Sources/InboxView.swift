import CoreSpotlight
import FolioKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct InboxView: View {
    let serverURL: URL
    let service: any FolioService
    let vaultID: UUID
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

    @Environment(\.undoManager) private var undoManager
    @State private var trashHistory: DocumentTrashHistory
    @State private var uploads: UploadViewModel
    @State private var showFileImporter = false
    @State private var showReviewFilters = false
    @State private var isDropTargeted = false
    @State private var isImporting = false
    @State private var showPhotosPicker = false
    @State private var photoSelection: [PhotosPickerItem] = []
    @State private var pendingDelete: Document?
    @State private var showScanner = false
    @State private var problem: String?
    @State private var scannedPages: ScannedPages?
    @State private var pendingUploads: UploadBatch?
    @State private var pageTask: Task<Void, Never>?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    #if os(iOS)
        @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif
    @FocusState private var searchFocused: Bool
    private let router = IntentRouter.shared

    init(
        serverURL: URL, service: any FolioService, session: DocumentBrowserSession,
        vaultID: UUID,
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
        _trashHistory = State(initialValue: DocumentTrashHistory(list: list))
        _uploads = State(
            initialValue: UploadViewModel(service: service) {
                Task { await list.load(afterUpload: true) }
            }
        )
    }

    var body: some View {
        GeometryReader { geometry in
            let showsPreview = geometry.size.width >= 760
            NavigationStack(
                path: Binding(
                    get: { showsPreview ? [] : session.path },
                    set: {
                        if !showsPreview {
                            session.path = $0
                        }
                    }
                )
            ) {
                HStack(spacing: 0) {
                    sidebar(showsPreview: showsPreview)
                        .frame(maxWidth: showsPreview ? 320 : .infinity)
                    if showsPreview {
                        Divider()
                        Group {
                            if let id = session.selection {
                                documentDestination(id)
                            } else {
                                ContentUnavailableView("Select a document", systemImage: "doc.text")
                            }
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
                .navigationDestination(for: Document.ID.self) { id in
                    documentDestination(id)
                }
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
        .onDisappear {
            pageTask?.cancel()
            trashHistory.invalidate()
        }
        .focusedSceneValue(\.inboxActions, actions)
        .photosPicker(isPresented: $showPhotosPicker, selection: $photoSelection, matching: .images)
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
        .telemetryScreen(
            list.showsSavedView ? TelemetryScreen.savedView : list.scope == .inbox ? TelemetryScreen.inbox : .documents)
    }

    private func documentDestination(_ id: Document.ID) -> some View {
        Group {
            if let document = list.document(for: id) {
                DocumentDetailView(
                    snapshot: .init(document: document, lookups: list.lookups), service: service,
                    vaultID: vaultID
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
            upload: canAcceptImport ? { showFileImporter = true } : nil,
            scan: scanAction,
            showInbox: { onNavigate(.inbox) },
            showAll: { onNavigate(.all) },
            find: {
                session.path = []
                searchFocused = true
            },
            deleteSelected: canEdit && !trashHistory.isWorking
                ? session.selection.flatMap { list.document(for: $0) }.map { document in
                    { pendingDelete = document }
                } : nil,
            photos: canAcceptImport ? { showPhotosPicker = true } : nil,
            signOut: onSignOut
        )
    }

    private var scanAction: (() -> Void)? {
        guard canAcceptImport else { return nil }
        #if os(iOS)
            guard DocumentScanner.isSupported else { return nil }
            return {
                FolioTelemetry.shared.action(.scanStart)
                showScanner = true
            }
        #else
            return nil
        #endif
    }

    private func handle(_ request: IntentRouter.Request) {
        switch request {
        case .scan:
            guard canAcceptImport else { return }
            if let scan = scanAction {
                scan()
            } else {
                // Macs and simulators have no scanner, so scanning falls back to choosing a file.
                showFileImporter = true
            }
        case .upload:
            guard canAcceptImport else { return }
            showFileImporter = true
        case .inbox:
            onNavigate(.inbox)
        case let .document(id, vaultID):
            FolioTelemetry.shared.action(.documentOpen)
            onOpenDocument(id, vaultID)
        }
    }

    private func sidebar(showsPreview: Bool) -> some View {
        @Bindable var list = session.list
        return Group {
            if session.presentation == .grid {
                documentGrid
            } else {
                List(selection: documentSelection) {
                    if list.phase == .loading && !list.rows.isEmpty {
                        ProgressView("Loading documents…")
                            .frame(maxWidth: .infinity)
                            .listRowBackground(Color.paper)
                            .listRowSeparator(.hidden)
                    }
                    ForEach(list.rows) { row in
                        documentLink(row, showsPreview: showsPreview)
                            .transition(.opacity)
                            .tag(row.id)
                            .listRowBackground(row.id == session.selection ? Color.archiveTint : Color.paper)
                            .listRowSeparatorTint(.rule)
                            .swipeActions {
                                if canEdit {
                                    Button("Move to Trash", systemImage: "trash", role: .destructive) {
                                        pendingDelete = row.document
                                    }
                                }
                            }
                    }
                    if list.phase != .loading && list.canLoadMore {
                        VStack(spacing: 8) {
                            if let message = list.pageError {
                                Text(message).foregroundStyle(Color.danger)
                            }
                            if list.isLoadingMore {
                                ProgressView("Loading documents…")
                            } else {
                                Button(list.pageError == nil ? "Load more documents" : "Retry loading documents") {
                                    pageTask?.cancel()
                                    FolioTelemetry.shared.action(.browseLoadMore)
                                    pageTask = Task { await list.loadNextPage() }
                                }
                                .buttonStyle(.borderless)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .listRowBackground(Color.paper)
                    }
                }
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.2), value: list.rowChangeRevision)
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.paper)
        .navigationTitle(session.savedViewName ?? (list.scope == .inbox ? "Inbox" : "All documents"))
        .searchable(text: $list.searchText, prompt: "Search documents")
        .searchFocused($searchFocused)
        .refreshable {
            FolioTelemetry.shared.action(.browseRefresh)
            await list.load()
        }
        .overlay { placeholder }
        .overlay {
            if isDropTargeted && canAcceptImport {
                ContentUnavailableView(
                    "Drop files to upload", systemImage: "tray.and.arrow.down",
                    description: Text("Images and PDFs.")
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.archiveTint)
                .allowsHitTesting(false)
            }
        }
        .onDrop(of: [UTType.fileURL], isTargeted: $isDropTargeted, perform: receiveDrop)
        .safeAreaInset(edge: .top, spacing: 0) {
            VStack(spacing: 0) {
                if case let .failed(message) = list.phase, !list.rows.isEmpty {
                    reloadFailure(message)
                }
                if uploads.state != .idle {
                    uploadNotice.transition(.opacity)
                }
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.2), value: uploads.state != .idle)
        .toolbar { toolbar }
        .sheet(isPresented: $showReviewFilters) {
            DocumentReviewFilters(filter: list.filter) { list.filter = $0 }
        }
        #if os(iOS)
            .safeAreaInset(edge: .bottom) {
                if horizontalSizeClass == .compact { scanButton }
            }
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
                onFinished: { filed in
                    scannedPages.close(scan.id)
                    Task { await list.load(afterUpload: filed) }
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
                importFiles(urls)
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
            guard canAcceptImport else { return }
            isImporting = true
            Task {
                reviewUploads(await loadPhotos(items))
                isImporting = false
            }
        }
        .confirmationDialog(
            "Move this document to Trash?",
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
            Button("Move to Trash", role: .destructive) {
                Task {
                    if await trashHistory.moveToTrash(document, undoManager: undoManager) {
                        if session.selection == document.id {
                            session.selection = nil
                        }
                        session.path.removeAll { $0 == document.id }
                    }
                }
            }
            .disabled(trashHistory.isWorking)
            Button("Keep", role: .cancel) {}
        } message: { document in
            Text(
                """
                \(document.title) will stay in Trash until you delete it forever. \
                You can restore it from Trash in the web app.
                """
            )
            if undoManager != nil {
                Text("You can also undo this action.")
            }
        }
    }

    private var documentSelection: Binding<Document.ID?>? {
        #if os(macOS)
            Binding(
                get: { session.selection },
                set: { id in
                    session.selection = id
                    session.path = id.map { [$0] } ?? []
                }
            )
        #else
            nil
        #endif
    }

    private var documentGrid: some View {
        ScrollView {
            LazyVGrid(
                columns: [
                    GridItem(.adaptive(minimum: FolioTheme.Spacing.space12 * 4), spacing: FolioTheme.Spacing.space4)
                ],
                spacing: FolioTheme.Spacing.space4
            ) {
                ForEach(list.rows) { row in
                    Button {
                        session.selection = row.id
                        session.path = [row.id]
                    } label: {
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                            DocumentThumbnailView(document: row.document)
                                .frame(height: FolioTheme.Spacing.space12 * 3)
                            Text(row.title)
                                .font(FolioTheme.Typography.heading)
                                .foregroundStyle(Color.ink)
                                .lineLimit(2)
                            Text(row.subtitle ?? row.dateText)
                                .font(FolioTheme.Typography.bodySm)
                                .foregroundStyle(Color.inkMuted)
                                .lineLimit(1)
                        }
                        .padding(FolioTheme.Spacing.space3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(row.id == session.selection ? Color.archiveTint : Color.paperRaised)
                        .clipShape(RoundedRectangle(cornerRadius: FolioTheme.Radius.radiusSm))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("document-\(row.id)")
                    .contextMenu {
                        if canEdit {
                            Button("Move to Trash", systemImage: "trash", role: .destructive) {
                                pendingDelete = row.document
                            }
                        }
                    }
                }
            }
            .padding(FolioTheme.Spacing.space4)
            if let message = list.pageError { Text(message).foregroundStyle(Color.danger) }
            if list.canLoadMore {
                Button(list.pageError == nil ? "Load more documents" : "Retry loading documents") {
                    pageTask?.cancel()
                    FolioTelemetry.shared.action(.browseLoadMore)
                    pageTask = Task { await list.loadNextPage() }
                }
                .disabled(list.isLoadingMore)
                .padding(FolioTheme.Spacing.space4)
            }
        }
    }

    private func documentLink(_ row: DocumentRow, showsPreview: Bool) -> some View {
        Button {
            session.selection = row.id
            session.path = [row.id]
        } label: {
            HStack {
                DocumentRowView(row: row)
                if !showsPreview {
                    Image(systemName: "chevron.right")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Color.inkMuted)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("document-\(row.id)")
    }

    @ViewBuilder
    private var placeholder: some View {
        switch list.phase {
        case .loading where list.rows.isEmpty,
            .idle where list.rows.isEmpty:
            Text("Reading your inbox…")
                .font(FolioTheme.Typography.body)
                .foregroundStyle(Color.inkMuted)
        case let .failed(message) where list.rows.isEmpty:
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
                        ? "Scan a letter, choose files, or drop images and PDFs here."
                        : "Try a different word."
                )
            }
        default:
            EmptyView()
        }
    }

    private func reloadFailure(_ message: String) -> some View {
        HStack {
            Label("Couldn't refresh. \(message)", systemImage: "exclamationmark.triangle")
            Spacer()
            Button("Try again") { Task { await list.load() } }
                .buttonStyle(.borderless)
        }
        .font(FolioTheme.Typography.bodySm)
        .foregroundStyle(Color.danger)
        .padding(FolioTheme.Spacing.space3)
        .background(Color.dangerTint)
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
            if list.supportsReviewFilters || list.filter.requiresReviewSupport {
                Button(
                    list.filter.requiresReviewSupport ? "Filters applied" : "Filters",
                    systemImage: "line.3.horizontal.decrease"
                ) { showReviewFilters = true }
            }
            #if os(iOS)
                Menu("Add", systemImage: "plus") {
                    if DocumentScanner.isSupported {
                        Button("Scan document", systemImage: "doc.viewfinder") { showScanner = true }
                    }
                    Button("Choose files", systemImage: "tray.and.arrow.down") { showFileImporter = true }
                    Button("Photos", systemImage: "photo") { showPhotosPicker = true }
                }
                .disabled(!canAcceptImport)
            #else
                Button("Choose files", systemImage: "tray.and.arrow.down") { showFileImporter = true }
                    .help("Upload files (\u{2318}O)")
                    .disabled(!canAcceptImport)
                Button("Photos", systemImage: "photo") { showPhotosPicker = true }
                    .disabled(!canAcceptImport)
            #endif
            Menu("Server", systemImage: "gearshape") {
                Text(serverURL.absoluteString)
                Button("Sign out", action: onSignOut)
            }
        }
    }

    #if os(iOS)
        private var scanButton: some View {
            Button {
                FolioTelemetry.shared.action(.scanStart)
                showScanner = true
            } label: {
                Label("Scan", systemImage: "doc.viewfinder").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(FolioTheme.Spacing.space4)
            .disabled(!canAcceptImport || !DocumentScanner.isSupported)
        }
    #endif

    private func reviewUploads(_ files: [UploadFile]) {
        guard !files.isEmpty else { return }
        let list = session.list
        uploads = UploadViewModel(service: service, destination: makeUploadDestination()) {
            Task { await list.load(afterUpload: true) }
        }
        pendingUploads = UploadBatch(files: files)
    }

    private var canAcceptImport: Bool {
        guard canUpload, !isImporting, pendingUploads == nil, scannedPages == nil, !showScanner else { return false }
        if case .uploading = uploads.state {
            return false
        }
        return true
    }

    private func receiveDrop(_ providers: [NSItemProvider]) -> Bool {
        guard !providers.isEmpty, canAcceptImport else { return false }
        isImporting = true
        Task {
            var urls: [URL] = []
            var failed = false
            for provider in providers {
                let url: URL? = await withCheckedContinuation { continuation in
                    _ = provider.loadDataRepresentation(forTypeIdentifier: UTType.fileURL.identifier) { data, _ in
                        continuation.resume(returning: data.flatMap { URL(dataRepresentation: $0, relativeTo: nil) })
                    }
                }
                if let url {
                    urls.append(url)
                } else {
                    failed = true
                }
            }
            await prepareFiles(urls)
            if failed {
                problem = "Couldn't read some dropped files. Try choosing them with Choose files."
            }
            isImporting = false
        }
        return true
    }

    private func importFiles(_ urls: [URL]) {
        guard canAcceptImport else {
            problem = "Wait for the current upload to finish, then try again."
            return
        }
        isImporting = true
        Task {
            await prepareFiles(urls)
            isImporting = false
        }
    }

    private func prepareFiles(_ urls: [URL]) async {
        let loaded = await UploadFile.readDocuments(urls)
        if !loaded.failed.isEmpty {
            problem = "Couldn't read \(loaded.failed.joined(separator: ", ")). Choose PDF or image files."
        }
        reviewUploads(loaded.files)
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
