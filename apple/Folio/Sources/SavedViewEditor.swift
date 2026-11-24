import FolioKit
import SwiftUI

struct SavedViewEditor: View {
    @Bindable var workspace: WorkspaceViewModel
    let existing: SavedView?
    @Environment(\.dismiss) private var dismiss
    @State private var draft: SavedViewDraft
    @State private var useCurrentQuery = false
    @State private var customFields: [CustomFieldDefinition] = []
    @State private var assignedValues: [UUID: CustomFieldValue] = [:]
    @State private var customFieldsError: String?
    @State private var exactReference: (filter: ExactCustomFieldFilter, label: String)?

    init(workspace: WorkspaceViewModel, existing: SavedView?) {
        self.workspace = workspace
        self.existing = existing
        _draft = State(
            initialValue: existing.map(SavedViewDraft.init)
                ?? SavedViewDraft(
                    query: workspace.currentSavedViewQuery, presentation: workspace.session.presentation
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
                    if !useCurrentQuery {
                        customFieldControls
                    }
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
                        if useCurrentQuery { update.query = workspace.currentSavedViewQuery }
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
        .task { await loadCustomFields() }
        .telemetryScreen(.savedViewEditor)
    }

    @ViewBuilder private var customFieldControls: some View {
        if let exact = draft.query.filter.metadataExact {
            let name = customFields.first { $0.id == exact.fieldID }?.draft.name ?? "Custom field"
            LabeledContent("Exact match") {
                VStack(alignment: .trailing) {
                    Text(name)
                    Text(exactValueLabel(exact))
                }
            }
            .task(id: exact) { await loadExactReference(exact) }
            Button("Remove custom field filter") {
                draft.query.filter.metadataExact = nil
            }
        }
        let choices = customFields.filter { assignedValues[$0.id] != nil && !$0.isRetired }
        if !choices.isEmpty {
            Menu("Match a custom field from this document") {
                ForEach(choices) { field in
                    if let value = assignedValues[field.id] {
                        Button(field.draft.name) {
                            draft.query.filter.metadataExact = .init(fieldID: field.id, value: value)
                        }
                    }
                }
            }
        }
        if let customFieldsError {
            Text(customFieldsError).foregroundStyle(Color.danger)
        }
    }

    private func exactValueLabel(_ exact: ExactCustomFieldFilter) -> String {
        switch exact.value {
        case let .text(value), let .longText(value): value.isEmpty ? "Empty text" : value
        case let .boolean(value): value ? "Yes" : "No"
        case let .integer(value), let .decimal(value), let .date(value), let .url(value): value
        case let .money(amount, currency): "\(amount) \(currency)"
        case .select, .documentLink:
            if let exactReference, exactReference.filter == exact {
                exactReference.label
            } else {
                "Loading…"
            }
        }
    }

    private func loadExactReference(_ exact: ExactCustomFieldFilter) async {
        let label: String
        switch exact.value {
        case let .select(id):
            let metadata = workspace.service as? any CustomMetadataService
            if let option = try? await metadata?.customOption(fieldID: exact.fieldID, id: id) {
                label = option.isRetired ? "\(option.label) (retired)" : option.label
            } else {
                label = "Option unavailable"
            }
        case let .documentLink(id):
            if let document = try? await workspace.service.document(id: id) {
                label = document.deletedAt == nil ? document.title : "\(document.title) (in Trash)"
            } else {
                label = "Document unavailable"
            }
        default:
            return
        }
        guard !Task.isCancelled else { return }
        exactReference = (exact, label)
    }

    private func loadCustomFields() async {
        guard let service = workspace.service as? any CustomMetadataService else { return }
        let document = workspace.session.selection ?? workspace.session.path.last
        do {
            let assignments: [CustomFieldAssignment]
            if let document {
                assignments = try await service.customMetadata(documentID: document).values
            } else {
                assignments = []
            }
            let values = Dictionary(
                uniqueKeysWithValues: assignments.compactMap { assignment in
                    assignment.value.map { (assignment.fieldID, $0) }
                })
            var ids = Set(values.keys)
            if let exact = draft.query.filter.metadataExact {
                ids.insert(exact.fieldID)
            }
            var definitions: [CustomFieldDefinition] = []
            var after: UUID?
            while !ids.isEmpty {
                let page = try await service.customFields(after: after)
                try Task.checkCancellation()
                definitions += page.filter { ids.remove($0.id) != nil }
                guard let last = page.last?.id, last != after else { break }
                after = last
            }
            try Task.checkCancellation()
            assignedValues = values
            customFields = definitions
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            customFieldsError = "Couldn’t load custom fields."
        }
    }

    private var summary: String {
        let query = useCurrentQuery ? workspace.currentSavedViewQuery : draft.query
        let scope = query.filter.inboxOnly ? "Inbox documents" : "All matching documents"
        let search = query.text.isEmpty ? scope : "\(scope). Search: \(query.text)"
        let count =
            query.filter.tagIDs.count
            + (query.filter.correspondentID == nil ? 0 : 1)
            + (query.filter.documentTypeID == nil ? 0 : 1)
            + (query.filter.createdAfter == nil ? 0 : 1)
            + (query.filter.createdBefore == nil ? 0 : 1)
            + (query.filter.metadataExact == nil ? 0 : 1)
        return count == 0 ? search : "\(search). \(count) active filters."
    }
}
