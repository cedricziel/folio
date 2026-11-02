import FolioKit
import SwiftUI

struct WorkspaceView: View {
    let serverURL: URL
    let service: any FolioService
    let onSignOut: () -> Void
    @State private var workspace: WorkspaceViewModel

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
                Section("Library") {
                    ForEach(WorkspaceDestination.allCases, id: \.self) { destination in
                        NavigationLink(value: destination) {
                            Label(destination.title, systemImage: destination.systemImage)
                        }
                    }
                }
            }
            .navigationTitle("Folio")
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 280)
            .disabled(!workspace.isReady)
        } detail: {
            if workspace.isReady {
                InboxView(
                    serverURL: serverURL, service: workspace.service, session: workspace.session,
                    vaultID: workspace.selectedVaultID,
                    canEdit: workspace.canEdit, canUpload: workspace.canUpload,
                    makeUploadDestination: workspace.uploadDestination,
                    onSignOut: onSignOut,
                    onNavigate: { workspace.destination = $0 },
                    onOpenDocument: { workspace.open($0, vaultID: $1) }
                )
                .id("\(workspace.selectedVaultID?.uuidString ?? "legacy")-\(workspace.destination.rawValue)")
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
        .task { await workspace.loadVaults() }
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
