import FolioKit
import SwiftUI

struct CustomFieldsManager: View {
    let metadata: any CustomMetadataService
    let canManage: Bool
    @State private var fields: [CustomFieldDefinition] = []
    @State private var after: UUID?
    @State private var loading = false
    @State private var loaded = false
    @State private var error: String?
    @State private var creating = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                ForEach(fields.sorted { $0.draft.name.localizedStandardCompare($1.draft.name) == .orderedAscending }) {
                    field in
                    NavigationLink {
                        CustomFieldSettings(
                            metadata: metadata, field: field, canManage: canManage,
                            onChange: { store($0) }
                        )
                    } label: {
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                            Text(field.draft.name)
                            Text(field.isRetired ? "\(field.draft.kind.label) · Retired" : field.draft.kind.label)
                                .foregroundStyle(Color.inkMuted)
                        }
                    }
                }
                if loading { ProgressView("Loading fields…") }
                if let error {
                    Text(error).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load() } }
                } else if after != nil {
                    Button("More fields") { Task { await load() } }
                } else if loaded && fields.isEmpty {
                    Text("This vault has no custom fields.").foregroundStyle(Color.inkMuted)
                }
            }
            .navigationTitle("Fields")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                if canManage {
                    ToolbarItem(placement: .primaryAction) {
                        Button("Add field", systemImage: "plus") { creating = true }
                    }
                }
            }
            .sheet(isPresented: $creating) {
                CustomFieldCreateView(metadata: metadata, onSave: store)
            }
            .task { if !loaded { await load() } }
        }
    }

    private func load() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            let page = try await metadata.customFields(after: after)
            try Task.checkCancellation()
            for field in page { store(field) }
            after = page.count == 50 ? page.last?.id : nil
            loaded = true
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    private func store(_ field: CustomFieldDefinition) {
        if let index = fields.firstIndex(where: { $0.id == field.id }) {
            fields[index] = field
        } else {
            fields.append(field)
        }
    }
}

private struct CustomFieldForm: View {
    @Binding var draft: CustomFieldDraft
    let choosingKind: Bool

    var body: some View {
        TextField("Name", text: $draft.name)
        if choosingKind {
            Picker("Type", selection: $draft.kind) {
                ForEach(CustomFieldKind.allCases) { kind in
                    Text(kind.label).tag(kind)
                }
            }
        } else {
            LabeledContent("Type", value: draft.kind.label)
        }
        Toggle("Required", isOn: $draft.required)
        Toggle("Unique in this vault", isOn: $draft.unique)
    }
}

private struct CustomFieldCreateView: View {
    let metadata: any CustomMetadataService
    let onSave: (CustomFieldDefinition) -> Void
    @State private var draft = CustomFieldDraft()
    @State private var error: String?
    @State private var saving = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                CustomFieldForm(draft: $draft, choosingKind: true)
                if let error { Text(error).foregroundStyle(Color.danger) }
            }
            .navigationTitle("New field")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(saving) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { Task { await save() } }
                        .disabled(saving || draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .interactiveDismissDisabled(saving)
        }
    }

    private func save() async {
        guard !saving else { return }
        saving = true
        defer { saving = false }
        do {
            let field = try await metadata.saveCustomField(id: nil, draft: draft)
            onSave(field)
            dismiss()
        } catch { self.error = error.localizedDescription }
    }
}

private struct CustomFieldSettings: View {
    let metadata: any CustomMetadataService
    let canManage: Bool
    let onChange: (CustomFieldDefinition) -> Void
    @State private var field: CustomFieldDefinition
    @State private var draft: CustomFieldDraft
    @State private var options: [CustomFieldOption] = []
    @State private var after: UUID?
    @State private var optionsError: String?
    @State private var loading = false
    @State private var saving = false
    @State private var error: String?
    @State private var optionEditor: OptionDraft?

    init(
        metadata: any CustomMetadataService, field: CustomFieldDefinition, canManage: Bool,
        onChange: @escaping (CustomFieldDefinition) -> Void
    ) {
        self.metadata = metadata
        self.canManage = canManage
        self.onChange = onChange
        _field = State(initialValue: field)
        _draft = State(initialValue: field.draft)
    }

    var body: some View {
        Form {
            Section("Field settings") {
                if canManage && !field.isRetired {
                    CustomFieldForm(draft: $draft, choosingKind: false)
                    Button("Save field") {
                        mutate { try await metadata.saveCustomField(id: field.id, draft: draft) }
                    }
                    .disabled(saving || draft == field.draft)
                } else {
                    LabeledContent("Type", value: field.draft.kind.label)
                    if field.draft.required { Text("Required") }
                    if field.draft.unique { Text("Unique in this vault") }
                    if field.isRetired { Text("Retired field") }
                }
                if canManage {
                    Button(field.isRetired ? "Restore field" : "Retire field") {
                        mutate {
                            try await metadata.setCustomFieldRetired(id: field.id, retired: !field.isRetired)
                            return .init(id: field.id, draft: field.draft, isRetired: !field.isRetired)
                        }
                    }
                    .disabled(saving)
                }
                if let error { Text(error).foregroundStyle(Color.danger) }
            }
            if field.draft.kind == .select {
                Section("Options") {
                    ForEach(options.sorted { ($0.position, $0.id.uuidString) < ($1.position, $1.id.uuidString) }) {
                        option in
                        HStack {
                            LabeledContent(
                                option.isRetired ? "\(option.label) (retired)" : option.label,
                                value: "Position \(option.position)"
                            )
                            if canManage && !field.isRetired {
                                if !option.isRetired {
                                    Button("Edit \(option.label)") { optionEditor = OptionDraft(option) }
                                }
                                Button(option.isRetired ? "Restore" : "Retire") {
                                    changeOptionLifecycle(option)
                                }
                                .disabled(saving)
                            }
                        }
                        .buttonStyle(.borderless)
                    }
                    if canManage && !field.isRetired {
                        Button("Add option") { optionEditor = OptionDraft(nil) }
                    }
                    if loading { ProgressView("Loading options…") }
                    if let optionsError {
                        Text(optionsError).foregroundStyle(Color.danger)
                        Button("Retry options") { Task { await loadOptions() } }
                    } else if after != nil {
                        Button("More options") { Task { await loadOptions() } }
                    }
                }
            }
        }
        .navigationTitle(field.draft.name)
        .task { if field.draft.kind == .select && options.isEmpty { await loadOptions() } }
        .sheet(item: $optionEditor) { draft in
            CustomOptionEditor(draft: draft) { label, position in
                let updated = try await metadata.saveCustomOption(
                    fieldID: field.id, id: draft.optionID, label: label, position: position
                )
                store(updated)
            }
        }
    }

    private func mutate(_ work: @escaping () async throws -> CustomFieldDefinition) {
        guard !saving else { return }
        saving = true
        error = nil
        Task {
            defer { saving = false }
            do {
                let updated = try await work()
                field = updated
                draft = updated.draft
                onChange(updated)
            } catch { self.error = error.localizedDescription }
        }
    }

    private func changeOptionLifecycle(_ option: CustomFieldOption) {
        guard !saving else { return }
        saving = true
        optionsError = nil
        Task {
            defer { saving = false }
            do {
                try await metadata.setCustomOptionRetired(fieldID: field.id, id: option.id, retired: !option.isRetired)
                store(
                    .init(id: option.id, label: option.label, position: option.position, isRetired: !option.isRetired))
            } catch { optionsError = error.localizedDescription }
        }
    }

    private func store(_ option: CustomFieldOption) {
        if let index = options.firstIndex(where: { $0.id == option.id }) {
            options[index] = option
        } else {
            options.append(option)
        }
    }

    private func loadOptions() async {
        guard !loading else { return }
        loading = true
        optionsError = nil
        defer { loading = false }
        do {
            let next = try await metadata.customOptions(fieldID: field.id, after: after)
            try Task.checkCancellation()
            for option in next { store(option) }
            after = next.count == 50 ? next.last?.id : nil
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            optionsError = error.localizedDescription
        }
    }
}

private struct OptionDraft: Identifiable {
    let id = UUID()
    let optionID: UUID?
    var label: String
    var position: String

    init(_ option: CustomFieldOption?) {
        optionID = option?.id
        label = option?.label ?? ""
        position = String(option?.position ?? 0)
    }
}

private struct CustomOptionEditor: View {
    @State var draft: OptionDraft
    let onSave: (String, UInt32) async throws -> Void
    @State private var saving = false
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                TextField("Name", text: $draft.label)
                TextField("Position", text: $draft.position)
                if let error { Text(error).foregroundStyle(Color.danger) }
            }
            .navigationTitle(draft.optionID == nil ? "New option" : "Edit option")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(saving) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
                        .disabled(saving || draft.label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .interactiveDismissDisabled(saving)
        }
    }

    private func save() async {
        guard !saving else { return }
        guard let position = UInt32(draft.position) else {
            error = "Enter a whole-number position between 0 and 4294967295."
            return
        }
        saving = true
        defer { saving = false }
        do {
            try await onSave(draft.label, position)
            dismiss()
        } catch { self.error = error.localizedDescription }
    }
}
