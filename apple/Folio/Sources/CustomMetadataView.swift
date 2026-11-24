import FolioKit
import SwiftUI

struct CustomMetadataView: View {
    let metadata: any CustomMetadataService
    let documents: any FolioService
    let canEdit: Bool
    let openDocument: (UUID) -> Void
    @State private var model: CustomMetadataViewModel
    @State private var editing: CustomFieldDefinition?
    @Environment(\.dismiss) private var dismiss

    init(
        documentID: UUID, metadata: any CustomMetadataService, documents: any FolioService,
        canEdit: Bool, openDocument: @escaping (UUID) -> Void
    ) {
        self.metadata = metadata
        self.documents = documents
        self.canEdit = canEdit
        self.openDocument = openDocument
        _model = State(initialValue: CustomMetadataViewModel(documentID: documentID, service: metadata))
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.fields) { field in
                    let assignment = model.snapshot.values.first { $0.fieldID == field.id }
                    if !field.isRetired || assignment?.value != nil {
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                            Text(field.draft.name).font(FolioTheme.Typography.body)
                            if field.isRetired {
                                Text("Retired field").foregroundStyle(Color.inkMuted)
                            }
                            CustomValueDisplay(
                                field: field, assignment: assignment, metadata: metadata,
                                openDocument: { id in
                                    dismiss()
                                    openDocument(id)
                                }
                            )
                            if canEdit {
                                Button("Edit \(field.draft.name)") { editing = field }
                            }
                        }
                    }
                }
                if model.isLoading { ProgressView("Loading custom fields…") }
                if let error = model.error {
                    Text(error).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await model.load(more: model.hasMore) } }
                } else if model.hasMore {
                    Button("More fields") { Task { await model.load(more: true) } }
                } else if !model.isLoading && model.fields.isEmpty {
                    Text("This vault has no custom fields.").foregroundStyle(Color.inkMuted)
                }
            }
            .navigationTitle("Custom fields")
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .task { await model.load() }
            .sheet(item: $editing) { field in
                CustomValueEditor(
                    field: field, current: model.snapshot.values.first { $0.fieldID == field.id }?.value,
                    metadata: metadata, documents: documents, model: model
                )
            }
        }
    }
}

private struct CustomValueDisplay: View {
    let field: CustomFieldDefinition
    let assignment: CustomFieldAssignment?
    let metadata: any CustomMetadataService
    let openDocument: (UUID) -> Void
    @State private var option: CustomFieldOption?
    @State private var optionUnavailable = false

    var body: some View {
        switch assignment?.value {
        case let .documentLink(id):
            if let target = assignment?.link {
                if target.isTrashed {
                    Text("\(target.title) (in Trash)").foregroundStyle(Color.inkMuted)
                } else {
                    Button(target.title, systemImage: "link") { openDocument(id) }
                }
            } else {
                Text("Document unavailable")
            }
        case let .url(value):
            if let url = URL(string: value), ["http", "https"].contains(url.scheme ?? "") {
                Link(value, destination: url)
            } else {
                Text(value)
            }
        case let .select(id):
            Text(
                option.map { $0.isRetired ? "\($0.label) (retired)" : $0.label }
                    ?? (optionUnavailable ? "Option unavailable" : "Loading option…")
            )
            .task(id: id) {
                option = nil
                optionUnavailable = false
                let loaded = try? await metadata.customOption(fieldID: field.id, id: id)
                guard !Task.isCancelled else { return }
                option = loaded
                optionUnavailable = loaded == nil
            }
        case let .text(value), let .longText(value):
            Text(value.isEmpty ? "Empty text" : value).textSelection(.enabled)
        case let .integer(value), let .decimal(value), let .date(value):
            Text(value).textSelection(.enabled)
        case let .money(amount, currency):
            Text("\(amount) \(currency)").textSelection(.enabled)
        case let .boolean(value):
            Text(value ? "Yes" : "No")
        case nil:
            Text(field.draft.required && !field.isRetired ? "Required, not set" : "Not set")
                .foregroundStyle(Color.inkMuted)
        }
    }
}

private struct CustomValueEditor: View {
    let field: CustomFieldDefinition
    let current: CustomFieldValue?
    let metadata: any CustomMetadataService
    let documents: any FolioService
    let model: CustomMetadataViewModel
    @State private var draft: CustomFieldValueDraft
    @State private var error: String?
    @State private var saving: Task<Void, Never>?
    @Environment(\.dismiss) private var dismiss

    init(
        field: CustomFieldDefinition, current: CustomFieldValue?, metadata: any CustomMetadataService,
        documents: any FolioService, model: CustomMetadataViewModel
    ) {
        self.field = field
        self.current = current
        self.metadata = metadata
        self.documents = documents
        self.model = model
        _draft = State(initialValue: CustomFieldValueDraft(current))
    }

    var body: some View {
        NavigationStack {
            Form {
                if let error { Text(error).foregroundStyle(Color.danger) }
                if model.needsReload {
                    Button("Reload current value") {
                        Task {
                            do {
                                try await model.reload()
                                error = nil
                            } catch {
                                self.error = error.localizedDescription
                            }
                        }
                    }
                }
                if field.isRetired {
                    Text("Retired field")
                } else {
                    CustomFieldValueInput(field: field, draft: $draft, metadata: metadata, documents: documents)
                }
                if current != nil && (!field.draft.required || field.isRetired) {
                    Button("Clear value", role: .destructive) { save(nil) }
                        .disabled(model.isSaving || model.needsReload)
                }
            }
            .navigationTitle(field.draft.name)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                if !field.isRetired {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") {
                            if let value { save(value) }
                        }
                        .disabled(value == nil || model.needsReload || model.isSaving)
                    }
                }
            }
            .onDisappear { saving?.cancel() }
        }
    }

    private var value: CustomFieldValue? {
        draft.value(for: field.draft.kind)
    }

    private func save(_ value: CustomFieldValue?) {
        guard !model.isSaving else { return }
        error = nil
        saving = Task {
            do {
                try await model.save(fieldID: field.id, value: value)
                dismiss()
            } catch {
                guard !Task.isCancelled else { return }
                self.error = error.localizedDescription
            }
        }
    }
}
