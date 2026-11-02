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
                    set: { if let destination = $0 { workspace.destination = destination } }
                )
            ) {
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
        } detail: {
            InboxView(
                serverURL: serverURL, service: service, session: workspace.session,
                onSignOut: onSignOut,
                onNavigate: { workspace.destination = $0 },
                onOpenDocument: workspace.open
            )
            .id(workspace.destination)
        }
    }
}
