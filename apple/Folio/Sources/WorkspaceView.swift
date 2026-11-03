import FolioKit
import SwiftUI

struct WorkspaceView: View {
    let serverURL: URL
    let service: any FolioService
    let onSignOut: () -> Void
    @State private var showSavedViewEditor = false
    @State private var editingSavedView: SavedView?
    @State private var pendingViewDelete: SavedView?
    @State private var workspace: WorkspaceViewModel
    @State private var activityVault: Vault?

    init(
        serverURL: URL, service: any FolioService, onSignOut: @escaping () -> Void,
        workspace: WorkspaceViewModel? = nil
    ) {
        self.serverURL = serverURL
        self.service = service
        self.onSignOut = onSignOut
        _workspace = State(initialValue: workspace ?? WorkspaceViewModel(service: service))
    }

    var body: some View {
        NavigationSplitView {
            List(
                selection: Binding(
                    get: { Optional(workspace.destination) },
                    set: {
                        if let destination = $0 {
                            workspace.destination = destination
                        }
                    }
                )
            ) {
                if workspace.isReady {
                    Section {
                        vaultPicker
                    }
                }
                if workspace.selectedVault?.role == .owner, service is any SourceActivityService {
                    Section("Sources") {
                        Button {
                            activityVault = workspace.selectedVault
                        } label: {
                            Label("Ingestion activity", systemImage: "tray.and.arrow.down")
                        }
                    }
                }
                Section("Library") {
                    ForEach(WorkspaceDestination.allCases, id: \.self) { destination in
                        NavigationLink(value: destination) {
                            Label(destination.title, systemImage: destination.systemImage)
                        }
                    }
                }
                if workspace.isReady {
                    Section("Saved views") {
                        ForEach(workspace.savedViews) { view in
                            Button {
                                Task { await workspace.openSavedView(view.id) }
                            } label: {
                                Label(view.name, systemImage: view.visibility == .private ? "bookmark" : "person.2")
                            }
                            .contextMenu {
                                Button("Edit saved view") {
                                    editingSavedView = view
                                    showSavedViewEditor = true
                                }
                                Button("Delete saved view", role: .destructive) { pendingViewDelete = view }
                            }
                        }
                        if let message = workspace.savedViewsError {
                            Text(message).font(FolioTheme.Typography.bodySm).foregroundStyle(Color.danger)
                            Button("Try again") { Task { await workspace.loadSavedViews() } }
                        }
                        if workspace.isLoadingSavedViews { ProgressView() }
                        if workspace.hasMoreSavedViews {
                            Button("More saved views") { Task { await workspace.loadSavedViews(more: true) } }
                        }
                    }
                }
            }
            .navigationTitle("Folio")
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 280)
            .disabled(!workspace.isReady)
        } detail: {
            if workspace.isReady, let vaultID = workspace.selectedVaultID {
                InboxView(
                    serverURL: serverURL, service: workspace.service, session: workspace.session,
                    vaultID: vaultID,
                    canEdit: workspace.canEdit, canUpload: workspace.canUpload,
                    makeUploadDestination: workspace.uploadDestination,
                    onSignOut: onSignOut,
                    onNavigate: { workspace.destination = $0 },
                    onOpenDocument: { workspace.open($0, vaultID: $1) }
                )
                .environment(\.thumbnailCache, workspace.thumbnails)

                .id(workspace.session.id)
                .toolbar {
                    if let selected = workspace.savedViews.first(where: { $0.id == workspace.selectedSavedViewID }) {
                        ToolbarItem(placement: .primaryAction) {
                            Menu("Saved view", systemImage: "ellipsis.circle") {
                                Button("Edit saved view") {
                                    editingSavedView = selected
                                    showSavedViewEditor = true
                                }
                                Button("Delete saved view", role: .destructive) { pendingViewDelete = selected }
                            }
                        }
                    }
                    ToolbarItem(placement: .primaryAction) {
                        Button("Save current view", systemImage: "bookmark.badge.plus") {
                            editingSavedView = nil
                            showSavedViewEditor = true
                        }
                        .disabled(workspace.destination == .inbox)
                    }
                }
            } else if let error = workspace.discoveryError {
                ContentUnavailableView {
                    Label("Couldn't load vaults", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(error)
                } actions: {
                    Button("Try again") { Task { await workspace.loadVaults() } }
                    Button("Sign out", action: onSignOut)
                }
            } else {
                ProgressView("Loading vaults…")
            }
        }
        .sheet(item: $activityVault) { vault in
            if let activityService = service as? any SourceActivityService {
                SourceActivityView(vaultID: vault.id, service: activityService) { documentID in
                    activityVault = nil
                    workspace.open(documentID, vaultID: vault.id)
                }
            }
        }
        .task { await workspace.loadVaults() }
        .task(id: workspace.selectedVaultID) { await workspace.loadSavedViews() }
        .sheet(isPresented: $showSavedViewEditor) {
            SavedViewEditor(workspace: workspace, existing: editingSavedView)
        }
        .onChange(of: workspace.selectedVaultID) { _, _ in
            showSavedViewEditor = false
            editingSavedView = nil
            pendingViewDelete = nil
        }
        .confirmationDialog(
            "Delete saved view?",
            isPresented: Binding(
                get: { pendingViewDelete != nil }, set: { if !$0 { pendingViewDelete = nil } }
            ), presenting: pendingViewDelete
        ) { view in
            Button("Delete saved view", role: .destructive) {
                Task { await workspace.deleteSavedView(view.id) }
            }
        } message: { _ in
            Text("Your documents will remain in the library.")
        }
        .alert(
            "Document unavailable",
            isPresented: Binding(
                get: { workspace.navigationError != nil },
                set: {
                    if !$0 {
                        workspace.dismissNavigationError()
                    }
                }
            )
        ) {
            Button("OK") { workspace.dismissNavigationError() }
        } message: {
            Text(workspace.navigationError ?? "")
        }
    }

    @ViewBuilder private var vaultPicker: some View {
        if workspace.catalog.vaults.count > 1 {
            Picker(
                "Vault",
                selection: Binding(
                    get: { workspace.selectedVaultID },
                    set: {
                        if let id = $0 {
                            workspace.selectVault(id)
                        }
                    }
                )
            ) {
                ForEach(workspace.catalog.vaults) { vault in
                    Text("\(vault.name) · \(vault.role.title)").tag(Optional(vault.id))
                }
            }
        } else {
            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                Label(workspace.selectedVault?.name ?? "Library", systemImage: "archivebox")
                if let vault = workspace.selectedVault {
                    Text(vault.role.title)
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.inkMuted)
                }
            }
        }
    }
}
