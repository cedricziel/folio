import FolioKit
import SwiftUI

struct SavedViewEditor: View {
    @Bindable var workspace: WorkspaceViewModel
    let existing: SavedView?
    @Environment(\.dismiss) private var dismiss
    @State private var draft: SavedViewDraft
    @State private var useCurrentQuery = false

    init(workspace: WorkspaceViewModel, existing: SavedView?) {
        self.workspace = workspace
        self.existing = existing
        _draft = State(
            initialValue: existing.map(SavedViewDraft.init)
                ?? SavedViewDraft(
                    query: workspace.session.list.query, presentation: workspace.session.presentation
                ))
    }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Name", text: $draft.name)
                Picker("Visible to", selection: $draft.visibility) {
                    Text("Only me").tag(SavedView.Visibility.private)
                    if workspace.canEdit {
                        Text("Everyone in this vault").tag(SavedView.Visibility.shared)
                    }
                }
                Picker("Display", selection: $draft.presentation) {
                    ForEach(SavedView.Presentation.allCases, id: \.self) { presentation in
                        Text(presentation.title).tag(presentation)
                    }
                }
                Section("Documents") {
                    if existing != nil {
                        Toggle("Use the current search and filters", isOn: $useCurrentQuery)
                    }
                    Text(summary)
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.inkMuted)
                }
                if let message = workspace.savedViewsError {
                    Text(message).foregroundStyle(Color.danger)
                }
            }
            .navigationTitle(existing == nil ? "Save current view" : "Edit saved view")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        var update = draft
                        if useCurrentQuery { update.query = workspace.session.list.query }
                        Task {
                            if await workspace.saveView(update, replacing: existing?.id) { dismiss() }
                        }
                    }
                    .disabled(
                        draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || workspace.isSavingView)
                }
            }
            .disabled(workspace.isSavingView)
        }
    }

    private var summary: String {
        let query = useCurrentQuery ? workspace.session.list.query : draft.query
        let search = query.text.isEmpty ? "All matching documents" : "Search: \(query.text)"
        let count =
            query.filter.tagIDs.count
            + (query.filter.correspondentID == nil ? 0 : 1)
            + (query.filter.documentTypeID == nil ? 0 : 1)
            + (query.filter.createdAfter == nil ? 0 : 1)
            + (query.filter.createdBefore == nil ? 0 : 1)
        return count == 0 ? search : "\(search). \(count) active filters."
    }
}
