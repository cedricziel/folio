import SwiftUI

struct WorkflowActionEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var draft: WorkflowActionDraft?
    @State private var choice: WorkflowTriggerChoice?
    @State private var error: String?
    @State private var isSaving = false
    @State private var needsReload = false
    let workflow: WorkflowConfigurationSummary
    let service: any WorkflowActionEditingService
    let onReload: () async throws -> Void
    let onSaved: (WorkflowConfigurationSummary, WorkflowTriggerChoice) -> Void

    var body: some View {
        NavigationStack {
            Form {
                if needsReload { WorkflowChangedNotice(reload: onReload) }
                if let draft = Binding($draft), let choice {
                    WorkflowActionRows(
                        draft: draft, target: choice.target, vaultID: workflow.vaultID,
                        taxonomy: service as? any WorkflowActionTaxonomyService,
                        metadata: (service as? any WorkflowActionMetadataService)?
                            .workflowMetadataService(vaultID: workflow.vaultID)
                    )
                    .disabled(isSaving || needsReload)

                    if let message = draft.wrappedValue.validationMessage(for: choice.target) ?? error {
                        Text(message).foregroundStyle(Color.danger)
                    }
                } else if let error {
                    Text(error).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load() } }
                } else if !needsReload {
                    ProgressView("Loading actions…")
                }
            }
            .navigationTitle("Edit actions")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }.disabled(!canSave || isSaving)
                }
            }
            .task { await load() }
        }
        .interactiveDismissDisabled(isSaving)
    }

    private var canSave: Bool {
        guard !needsReload, let draft, let choice else { return false }
        return draft.validationMessage(for: choice.target) == nil
    }

    private func load() async {
        guard draft == nil else { return }
        error = nil
        do {
            async let choices = service.workflowTriggerChoices(vaultID: workflow.vaultID)
            async let loaded = service.loadWorkflowActions(workflow)
            guard let current = WorkflowTriggerChoice.matching(workflow.selectedTriggers, in: try await choices) else {
                throw FolioError.unreadableResponse
            }
            draft = try await loaded
            choice = current
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
        } catch {
            guard !Task.isCancelled else { return }
            self.error = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't load actions. Try again.")
        }
    }

    private func save() async {
        guard canSave, !isSaving, let draft, let choice else { return }
        isSaving = true
        error = nil
        defer { isSaving = false }
        do {
            let saved = try await service.saveWorkflowActions(workflow, actions: draft, choice: choice)
            onSaved(saved, choice)
            dismiss()
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
        } catch {
            self.error = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't save actions. Try again.")
        }
    }
}

struct WorkflowActionRows: View {
    @Binding var draft: WorkflowActionDraft
    let target: WorkflowTriggerChoice.Target
    let vaultID: UUID
    let taxonomy: (any WorkflowActionTaxonomyService)?
    let metadata: (any FolioService & CustomMetadataService)?

    var body: some View {
        Section("Actions") {
            ForEach(Array(draft.rows.enumerated()), id: \.element.id) { index, row in
                actionRow(row, position: index, draft: $draft)
            }
            NavigationLink("Add action") {
                WorkflowActionChoices(kinds: actionChoices) {
                    draft.rows.append(.init(kind: $0))
                }
            }
        }
    }

    private var actionChoices: [WorkflowActionDraft.Kind] {
        let available = WorkflowActionDraft.Kind.available(for: target)
        guard metadata == nil else { return available }
        return available.filter { $0 != .setMetadata && $0 != .clearMetadata }
    }

    @ViewBuilder private func actionRow(
        _ row: WorkflowActionDraft.Row, position: Int, draft: Binding<WorkflowActionDraft>
    ) -> some View {
        let title = "\(position + 1). \(row.kind.title)"
        if row.kind.hasFields {
            NavigationLink {
                WorkflowActionFields(
                    row: binding(row, in: draft), vaultID: vaultID, taxonomy: taxonomy, metadata: metadata
                )
                .navigationTitle(row.kind.title)
            } label: {
                VStack(alignment: .leading) {
                    Text(title)
                    if let destination = row.destination {
                        Text(destination).font(.subheadline).foregroundStyle(Color.inkMuted)
                    }
                }
            }
            .accessibilityLabel(row.destination.map { "\(title), \($0)" } ?? title)
            .accessibilityIdentifier("workflow-action-\(position + 1)")
        } else {
            Text(title).accessibilityIdentifier("workflow-action-\(position + 1)")
        }

        HStack {
            Button {
                draft.wrappedValue.move(row.id, by: -1)
            } label: {
                Image(systemName: "arrow.up")
            }
            .disabled(position == 0)
            .accessibilityLabel("Move action \(position + 1) up")

            Button {
                draft.wrappedValue.move(row.id, by: 1)
            } label: {
                Image(systemName: "arrow.down")
            }
            .disabled(position == draft.wrappedValue.rows.count - 1)
            .accessibilityLabel("Move action \(position + 1) down")

            Spacer()
            Button(role: .destructive) {
                draft.wrappedValue.rows.removeAll { $0.id == row.id }
            } label: {
                Image(systemName: "trash")
            }
            .accessibilityLabel("Remove action \(position + 1)")
        }
        .buttonStyle(.borderless)
    }

    private func binding(_ row: WorkflowActionDraft.Row, in draft: Binding<WorkflowActionDraft>)
        -> Binding<WorkflowActionDraft.Row>
    {
        Binding {
            draft.wrappedValue.rows.first(where: { $0.id == row.id }) ?? row
        } set: { updated in
            guard let index = draft.wrappedValue.rows.firstIndex(where: { $0.id == row.id }) else { return }
            draft.wrappedValue.rows[index] = updated
        }
    }
}

private struct WorkflowActionChoices: View {
    @Environment(\.dismiss) private var dismiss
    let kinds: [WorkflowActionDraft.Kind]
    let choose: (WorkflowActionDraft.Kind) -> Void

    var body: some View {
        List(kinds) { kind in
            Button(kind.title) {
                choose(kind)
                dismiss()
            }
        }
        .navigationTitle("Add action")
    }
}

private struct WorkflowActionFields: View {
    @Binding var row: WorkflowActionDraft.Row
    let vaultID: UUID
    let taxonomy: (any WorkflowActionTaxonomyService)?
    let metadata: (any FolioService & CustomMetadataService)?

    var body: some View {
        Form {
            switch row.kind {
            case .webhook:
                TextField("Webhook URL", text: $row.url).autocorrectionDisabled()
                LabeledContent("Method", value: "POST")
                SecureField("Signing secret", text: $row.signingSecret)
                Section("Headers") {
                    TextEditor(text: $row.headerLines).frame(minHeight: 96).accessibilityLabel("Webhook headers")
                    Text(
                        "Enter one Name: Value pair per line. Keep saved credentials masked when the URL stays the same."
                    )
                    .foregroundStyle(Color.inkMuted)
                }
            case .addTags, .removeTags:
                classification(.tags, fallback: "Tag IDs separated by commas")
            case .rename:
                TextField("Document title", text: $row.title)
            case .documentDate:
                TextField("Document date", text: $row.date, prompt: Text("YYYY-MM-DD"))
                    .autocorrectionDisabled()
                    .accessibilityLabel("Document date")
                    .accessibilityIdentifier("workflow-document-date")
            case .reviewed:
                Picker("Review state", selection: $row.reviewed) {
                    Text("Needs review").tag(false)
                    Text("Reviewed").tag(true)
                }
            case .setMetadata, .clearMetadata:
                if let metadata {
                    WorkflowMetadataActionFields(row: $row, source: metadata)
                } else {
                    Text("Custom field choices are unavailable.")
                }
            case .trash:
                Text("Place this action last.")
            case .clearCorrespondent, .clearDocumentType, .clearDocumentDate:
                EmptyView()
            case .correspondent:
                classification(.correspondents, fallback: "Correspondent ID")
            case .documentType:
                classification(.documentTypes, fallback: "Document type ID")
            case .skipImport:
                Text("Skip the matching file before importing it.")
            }
        }
    }

    @ViewBuilder private func classification(_ kind: TaxonomyKind, fallback: String) -> some View {
        if let taxonomy {
            WorkflowActionClassification(kind: kind, vaultID: vaultID, service: taxonomy, references: $row.references)
        } else {
            TextField(fallback, text: $row.references).autocorrectionDisabled()
        }
    }
}

private struct WorkflowActionClassification: View {
    let kind: TaxonomyKind
    let vaultID: UUID
    let service: any WorkflowActionTaxonomyService
    @Binding var references: String
    @State private var entries: [TaxonomyEntry]?
    @State private var error: String?
    @State private var isLoading = false

    var body: some View {
        Group {
            if let entries {
                choices(entries)
            } else if let error {
                Text(error).foregroundStyle(Color.danger)
                Button("Try again") { Task { await load() } }
            } else {
                ProgressView("Loading choices…")
            }
        }
        .task {
            if error == nil { await load() }
        }
    }

    @ViewBuilder private func choices(_ entries: [TaxonomyEntry]) -> some View {
        let available = Set(entries.map { $0.id.uuidString.lowercased() })
        let unavailable = selection.filter { !available.contains($0) }
        if entries.isEmpty && unavailable.isEmpty {
            Text("No \(kind.singular) choices in this vault.")
        } else if kind == .tags {
            ForEach(entries) { entry in
                Toggle(entry.name, isOn: tag(entry.id.uuidString.lowercased()))
            }
            ForEach(unavailable, id: \.self) { id in
                Toggle("Unavailable tag", isOn: tag(id))
            }
        } else {
            Picker("Select a \(kind.singular)", selection: $references) {
                Text("Choose a \(kind.singular)").tag("")
                ForEach(entries) { entry in
                    Text(entry.name).tag(entry.id.uuidString.lowercased())
                }
                if !references.isEmpty && !available.contains(references) {
                    Text("Unavailable \(kind.singular)").tag(references)
                }
            }
        }
    }

    private var selection: [String] {
        references.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    private func tag(_ id: String) -> Binding<Bool> {
        Binding {
            selection.contains(id)
        } set: { selected in
            var values = selection.filter { $0 != id }
            if selected { values.append(id) }
            references = values.joined(separator: ", ")
        }
    }

    private func load() async {
        guard entries == nil, !isLoading else { return }
        isLoading = true
        error = nil
        defer { isLoading = false }
        do {
            entries = try await service.workflowActionTaxonomy(kind, vaultID: vaultID)
                .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        } catch {
            guard !Task.isCancelled else { return }
            self.error = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't load choices. Try again.")
        }
    }
}

private extension WorkflowActionDraft.Kind {
    var hasFields: Bool {
        switch self {
        case .webhook, .addTags, .removeTags, .correspondent, .documentType, .rename, .documentDate, .reviewed,
            .skipImport, .trash, .setMetadata, .clearMetadata:
            true
        case .clearCorrespondent, .clearDocumentType, .clearDocumentDate:
            false
        }
    }
}

private struct WorkflowMetadataActionFields: View {
    @Binding var row: WorkflowActionDraft.Row
    let source: any FolioService & CustomMetadataService
    @State private var fields: [CustomFieldDefinition] = []
    @State private var error: String?
    @State private var loading = false

    var body: some View {
        Group {
            Picker("Custom field", selection: fieldSelection) {
                Text("Choose a custom field").tag(UUID?.none)
                if let saved = row.metadataFieldID, !available.contains(where: { $0.id == saved }) {
                    Text(fields.first(where: { $0.id == saved })?.draft.name ?? "Saved custom field")
                        .tag(Optional(saved))
                        .disabled(true)
                }
                ForEach(available) { field in
                    Text(field.draft.name).tag(Optional(field.id))
                }
            }
            .pickerStyle(.menu)
            .accessibilityIdentifier("workflow-custom-field")
            .disabled(loading)
            .task { await load() }
            if loading { ProgressView("Loading custom fields…") }
            if let error {
                Text(error).foregroundStyle(Color.danger)
                Button("Retry custom fields") { Task { await load() } }
            }
            if row.kind == .setMetadata, let field = available.first(where: { $0.id == row.metadataFieldID }) {
                if row.metadataKind == field.draft.kind {
                    CustomFieldValueInput(field: field, draft: $row.metadataValue, metadata: source, documents: source)
                        .id(field.id)
                } else {
                    Button("Use current field type") { select(field) }
                }
            }
        }
    }

    private var available: [CustomFieldDefinition] {
        fields.filter { !$0.isRetired && (row.kind != .clearMetadata || !$0.draft.required) }
    }

    private var fieldSelection: Binding<UUID?> {
        Binding {
            row.metadataFieldID
        } set: { id in
            guard id != row.metadataFieldID else { return }
            select(available.first(where: { $0.id == id }))
        }
    }

    private func select(_ field: CustomFieldDefinition?) {
        row.metadataFieldID = field?.id
        row.metadataKind = field?.draft.kind
        row.metadataValue = CustomFieldValueDraft()
    }

    private func load() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            var cursor: UUID?
            var loaded: [CustomFieldDefinition] = []
            while true {
                let page = try await source.customFields(after: cursor)
                try Task.checkCancellation()
                loaded.append(contentsOf: page)
                guard page.count == 50, let next = page.last?.id, next != cursor else { break }
                cursor = next
            }
            fields = loaded.sorted {
                let order = $0.draft.name.localizedStandardCompare($1.draft.name)
                return order == .orderedSame ? $0.id.uuidString < $1.id.uuidString : order == .orderedAscending
            }
        } catch {
            guard !Task.isCancelled else { return }
            self.error = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't load custom fields.")
        }
    }
}
