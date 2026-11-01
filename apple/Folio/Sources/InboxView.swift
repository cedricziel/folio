import FolioKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct InboxView: View {
    let serverURL: URL
    let service: any FolioService
    let onSignOut: () -> Void

    @State private var list: DocumentListViewModel
    @State private var uploads: UploadViewModel
    @State private var selection: Document.ID?
    @State private var showFileImporter = false
    @State private var photoSelection: [PhotosPickerItem] = []
    @State private var pendingDelete: Document?
    @State private var showScanner = false
    @State private var problem: String?
    @State private var scannedPages: ScannedPages?

    init(serverURL: URL, service: any FolioService, onSignOut: @escaping () -> Void) {
        self.serverURL = serverURL
        self.service = service
        self.onSignOut = onSignOut
        let list = DocumentListViewModel(service: service)
        _list = State(initialValue: list)
        _uploads = State(
            initialValue: UploadViewModel(service: service) {
                Task { await list.load() }
            }
        )
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            if let row = list.rows.first(where: { $0.id == selection }) {
                DocumentDetailView(
                    model: DocumentDetailViewModel(
                        document: row.document,
                        correspondent: list.correspondentName(for: row.document),
                        type: list.typeName(for: row.document),
                        service: service
                    )
                )
                .id(row.id)
            } else {
                ContentUnavailableView(
                    "Select a document", systemImage: "doc.text",
                    description: Text("Pick one from the inbox to read it here.")
                )
            }
        }
    }

    private var sidebar: some View {
        List(selection: $selection) {
            ForEach(list.rows) { row in
                DocumentRowView(row: row)
                    .tag(row.id)
                    .listRowBackground(row.id == selection ? Color.archiveTint : Color.paper)
                    .listRowSeparatorTint(.rule)
                    .swipeActions {
                        Button("Delete", systemImage: "trash", role: .destructive) {
                            pendingDelete = row.document
                        }
                    }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.paper)
        .navigationTitle("Inbox")
        .searchable(text: $list.searchText, prompt: "Search documents")
        .task(id: list.searchText) { await list.search(list.searchText) }
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
                model: ScanReviewViewModel(pages: scan.images, service: service),
                onFinished: {
                    scannedPages = nil
                    Task { await list.load() }
                }
            )
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
                    await uploads.upload(loaded.files)
                }
            }
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(
                get: { problem != nil || list.actionError != nil },
                set: { if !$0 { problem = nil; list.dismissActionError() } })
        ) {
            Button("OK") {}
        } message: {
            Text(problem ?? list.actionError ?? "")
        }
        .onChange(of: photoSelection) { _, items in
            guard !items.isEmpty else { return }
            photoSelection = []
            Task { await uploads.upload(await loadPhotos(items)) }
        }
        .confirmationDialog(
            "Delete this document?",
            isPresented: Binding(
                get: { pendingDelete != nil },
                set: {
                    if !$0 {
                        pendingDelete = nil
                    }
                }),
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
        case .loading,
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
            PhotosPicker(selection: $photoSelection, matching: .images) {
                Label("Photos", systemImage: "photo")
            }
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
            .disabled(!DocumentScanner.isSupported)
        }
    #endif

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

struct ScannedPages: Identifiable {
    let id = UUID()
    let images: [CGImage]
}
