import SwiftUI

struct CustomMetadataWorkflowEditor: View {
    let workflows: any CustomMetadataWorkflowService
    let metadata: any CustomMetadataService
    @State private var triggers: [CustomMetadataWorkflowTrigger] = []
    @State private var fields: [CustomFieldDefinition] = []
    @State private var fieldCursor: UUID?
    @State private var trigger = ""
    @State private var fieldID: UUID?
    @State private var kind: CustomFieldKind?
    @State private var state: CustomMetadataChangeState?
    @State private var before = ConditionValue()
    @State private var after = ConditionValue()
    @State private var name = ""
    @State private var webhook = ""
    @State private var enabled = true
    @State private var loading = false
    @State private var loadingFields = false
    @State private var saving = false
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    private var selectedTrigger: CustomMetadataWorkflowTrigger? {
        triggers.first { $0.id == trigger }
    }

    var body: some View {
        NavigationStack {
            Form {
                if loading { ProgressView("Loading workflow triggers…") }
                if let error { Text(error).foregroundStyle(Color.danger) }
                if !loading && triggers.isEmpty {
                    if error == nil { Text("Custom field workflows are unavailable on this server.") }
                    Button("Try again") { Task { await load() } }
                } else {
                    TextField("Name", text: $name)
                    Picker("When", selection: $trigger) {
                        Text("Choose a trigger").tag("")
                        ForEach(triggers) { row in Text(row.label).tag(row.id) }
                    }
                    .pickerStyle(.menu)
                    .accessibilityIdentifier("metadata-workflow-trigger")
                    if let selectedTrigger {
                        conditions(supportsValues: selectedTrigger.supportsValues)
                    }
                    Section("Send a webhook") {
                        TextField("Destination URL", text: $webhook)
                        Toggle("Enabled", isOn: $enabled)
                    }
                }
            }
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle("New custom field workflow")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(saving) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save workflow") { Task { await save() } }
                        .disabled(saving || selectedTrigger == nil || name.isEmpty || webhook.isEmpty)
                }
            }
            .interactiveDismissDisabled(saving)
            .task { await load() }
        }
    }

    private func conditions(supportsValues: Bool) -> some View {
        Section("Custom field conditions") {
            Picker("Custom field", selection: $fieldID) {
                Text("Any custom field").tag(UUID?.none)
                ForEach(fields.sorted { $0.draft.name.localizedStandardCompare($1.draft.name) == .orderedAscending }) {
                    Text($0.draft.name).tag(Optional($0.id))
                }
            }
            .pickerStyle(.menu)
            .accessibilityIdentifier("metadata-workflow-field")
            .onChange(of: fieldID) {
                if let field = fields.first(where: { $0.id == fieldID }) { kind = field.draft.kind }
            }
            if fieldCursor != nil {
                Button("More fields") { Task { await loadMoreFields() } }.disabled(loadingFields)
            }
            Picker("Field type", selection: $kind) {
                Text("Any type").tag(CustomFieldKind?.none)
                ForEach(CustomFieldKind.allCases) { Text($0.label).tag(Optional($0)) }
            }
            .disabled(fieldID != nil)
            if supportsValues {
                Picker("Change", selection: $state) {
                    Text("Any change").tag(CustomMetadataChangeState?.none)
                    ForEach(CustomMetadataChangeState.allCases) { Text($0.label).tag(Optional($0)) }
                }
                if let kind {
                    if state != .set {
                        ConditionValueEditor(label: "Previous value", kind: kind, draft: $before)
                    }
                    if state != .cleared {
                        ConditionValueEditor(label: "New value", kind: kind, draft: $after)
                    }
                }
            }
        }
    }

    private func load() async {
        guard !loading else { return }
        loading = true
        defer { loading = false }
        do {
            async let catalog = workflows.customMetadataWorkflowTriggers()
            async let definitions = metadata.customFields(after: nil)
            let (available, page) = try await (catalog, definitions)
            try Task.checkCancellation()
            triggers = available
            fields = page.filter { !$0.isRetired }
            fieldCursor = page.count == 50 ? page.last?.id : nil
            error = nil
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    private func loadMoreFields() async {
        guard !loadingFields, let fieldCursor else { return }
        loadingFields = true
        defer { loadingFields = false }
        do {
            let page = try await metadata.customFields(after: fieldCursor)
            try Task.checkCancellation()
            fields += page.filter { !$0.isRetired }
            self.fieldCursor = page.count == 50 ? page.last?.id : nil
            error = nil
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    private func save() async {
        guard !saving, let selection = selectedTrigger else { return }
        let supportsValues = selection.supportsValues
        let compareBefore = supportsValues && state != .set && before.matches
        let compareAfter = supportsValues && state != .cleared && after.matches
        let from = compareBefore ? kind.flatMap(before.value) : nil
        let to = compareAfter ? kind.flatMap(after.value) : nil
        if (compareBefore && from == nil) || (compareAfter && to == nil) {
            error = "Enter a valid value for each comparison."
            return
        }
        saving = true
        error = nil
        defer { saving = false }
        do {
            try await workflows.createCustomMetadataWorkflow(
                .init(
                    name: name, trigger: selection.id, fieldID: fieldID, kind: kind,
                    state: supportsValues ? state : nil, before: from, after: to, webhook: webhook, enabled: enabled
                ))
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

private struct ConditionValue {
    var matches = false
    var text = ""
    var boolean: Bool?
    var currency = ""

    func value(_ kind: CustomFieldKind) -> CustomFieldValue? {
        switch kind {
        case .text: .text(text)
        case .longText: .longText(text)
        case .boolean: boolean.map(CustomFieldValue.boolean)
        case .integer: .integer(text)
        case .decimal: .decimal(text)
        case .date: .date(text)
        case .money: .money(amount: text, currency: currency)
        case .select: UUID(uuidString: text).map(CustomFieldValue.select)
        case .url: .url(text)
        case .documentLink: UUID(uuidString: text).map(CustomFieldValue.documentLink)
        }
    }
}

private struct ConditionValueEditor: View {
    let label: String
    let kind: CustomFieldKind
    @Binding var draft: ConditionValue

    var body: some View {
        Toggle("Match \(label.lowercased())", isOn: $draft.matches)
        if draft.matches {
            switch kind {
            case .boolean:
                Picker(label, selection: $draft.boolean) {
                    Text("Choose Yes or No").tag(Bool?.none)
                    Text("Yes").tag(Optional(true))
                    Text("No").tag(Optional(false))
                }
            case .money:
                TextField(label, text: $draft.text)
                TextField("\(label) currency code", text: $draft.currency)
            case .date:
                TextField("\(label) (YYYY-MM-DD)", text: $draft.text)
            case .select:
                TextField("\(label) option ID", text: $draft.text)
            case .documentLink:
                TextField("\(label) document ID", text: $draft.text)
            case .longText:
                TextField(label, text: $draft.text, axis: .vertical)
            default:
                TextField(label, text: $draft.text)
            }
        }
    }
}
