import SwiftUI

struct SourceLifecycleDetailView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model: SourceLifecycleViewModel
    @State private var outcomes: SourceOutcomesViewModel
    @State private var confirmingTrash = false
    private let onChanged: () -> Void
    private let openDocument: (UUID) -> Void

    init(
        source: SourceActivity, service: any SourceLifecycleService, canManage: Bool,
        onChanged: @escaping () -> Void, openDocument: @escaping (UUID) -> Void
    ) {
        _model = State(initialValue: SourceLifecycleViewModel(source: source, service: service, canManage: canManage))
        _outcomes = State(
            initialValue: SourceOutcomesViewModel(vaultID: source.vaultID, sourceID: source.id, service: service))
        self.onChanged = onChanged
        self.openDocument = openDocument
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space4) {
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                    Text(model.source.deletedAt != nil ? "In Trash" : model.source.health.title)
                        .foregroundStyle(model.source.health == .unavailable ? Color.danger : Color.ink)
                    if model.source.origin == .config {
                        Text("Configured on the server. Change this source in the server configuration.")
                            .foregroundStyle(Color.inkMuted)
                    } else if !model.source.supportsLifecycle {
                        Text("Source management is unavailable on this server.")
                            .foregroundStyle(Color.inkMuted)
                    }
                    if let date = model.source.lastCheckedAt { dateRow("Last checked", date: date) }
                    if let date = model.source.createdAt {
                        dateRow("Created", date: date, actor: model.source.createdBy ?? "System or unknown")
                    }
                    if let date = model.source.updatedAt {
                        dateRow("Updated", date: date, actor: model.source.updatedBy ?? "System or unknown")
                    }
                    if let date = model.source.deletedAt { dateRow("Moved to Trash", date: date) }
                }
                if model.canChange {
                    Button(model.source.enabled ? "Disable source" : "Enable source") {
                        Task { await model.setEnabled(!model.source.enabled) }
                    }
                    .disabled(model.isSaving)
                    Button("Move source to Trash", role: .destructive) { confirmingTrash = true }
                        .disabled(model.isSaving)
                } else if model.canRestore {
                    Text(
                        "Restoring keeps the previous setting. Enabled sources resume imports; disabled sources stay off."
                    )
                    .font(FolioTheme.Typography.bodySm)
                    .foregroundStyle(Color.inkMuted)
                    Button("Restore source") { Task { await model.restore() } }
                        .disabled(model.isSaving)
                }
                if model.isSaving { ProgressView("Saving source…") }
                if let error = model.error {
                    Text(error).foregroundStyle(Color.danger).accessibilityAddTraits(.updatesFrequently)
                }
                SourceOutcomesView(model: outcomes, openDocument: openDocument)
            }
            .padding(FolioTheme.Spacing.space4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(model.source.name)
        .refreshable { await outcomes.load() }
        .confirmationDialog("Move this source to Trash?", isPresented: $confirmingTrash, titleVisibility: .visible) {
            Button("Move to Trash", role: .destructive) { Task { await model.trash() } }
        } message: {
            Text(
                "New imports stop. Existing documents and import history stay in this vault. You can restore the source later."
            )
        }
        .onChange(of: model.source) { previous, current in
            onChanged()
            if previous.deletedAt != nil, current.deletedAt == nil { dismiss() }
        }
        .onChange(of: model.didTrash) {
            if model.didTrash { onChanged(); dismiss() }
        }
    }

    private func dateRow(_ label: String, date: Date, actor: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
            Text("\(label) \(date.formatted(date: .abbreviated, time: .shortened))")
            if let actor { Text("By \(actor)") }
        }
        .font(FolioTheme.Typography.bodySm)
        .foregroundStyle(Color.inkMuted)
    }
}
