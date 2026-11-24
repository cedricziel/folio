import SwiftUI

struct WorkflowTriggerPicker: View {
    private let choices: [WorkflowTriggerChoice]
    @Binding private var selection: String

    init(choices: [WorkflowTriggerChoice], selection: Binding<String>) {
        self.choices = choices
        _selection = selection
    }

    var body: some View {
        Picker("When", selection: $selection) {
            if !choices.contains(where: { $0.id == selection }) {
                Text(selection.isEmpty ? "Choose a trigger" : "Unavailable trigger").tag(selection)
            }
            ForEach(choices) { choice in
                Text(choice.title).tag(choice.id)
            }
        }
    }
}

struct WorkflowTriggerEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var choices: [WorkflowTriggerChoice] = []
    @State private var selection: String
    @State private var documentConditions: WorkflowDocumentConditionDraft?
    @State private var initialDocumentConditions: WorkflowDocumentConditionDraft?
    @State private var isLoading = false
    @State private var isSaving = false
    @State private var loadError: String?
    @State private var saveError: String?
    @State private var needsReload = false
    @State private var workflow: WorkflowConfigurationSummary
    private let service: any WorkflowTriggerEditingService
    private let onUpdated: (WorkflowConfigurationSummary, WorkflowTriggerChoice?) -> WorkflowConfigurationSummary?

    private var conditionService: (any WorkflowDocumentConditionEditingService)? {
        service as? any WorkflowDocumentConditionEditingService
    }

    private var selectedChoice: WorkflowTriggerChoice? {
        choices.first(where: { $0.id == selection })
    }

    private var hasChanges: Bool {
        selection != workflow.trigger || documentConditions != initialDocumentConditions
    }

    init(
        workflow: WorkflowConfigurationSummary, service: any WorkflowTriggerEditingService,
        onUpdated: @escaping (WorkflowConfigurationSummary, WorkflowTriggerChoice?) -> WorkflowConfigurationSummary?
    ) {
        _workflow = State(initialValue: workflow)
        self.service = service
        self.onUpdated = onUpdated
        _selection = State(initialValue: workflow.trigger)
    }

    var body: some View {
        NavigationStack {
            Form {
                if isLoading {
                    ProgressView(needsReload ? "Reloading workflow…" : "Loading triggers…")
                } else if let loadError {
                    Text(loadError).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load() } }
                } else if choices.isEmpty {
                    Text("No triggers available.").foregroundStyle(Color.inkMuted)
                } else {
                    WorkflowTriggerPicker(choices: choices, selection: $selection)
                        .pickerStyle(.inline)
                        .disabled(isSaving || needsReload)
                    if let conditions = documentConditions {
                        Section {
                            ForEach(conditions.fields) { field in
                                conditionField(field)
                            }
                        } header: {
                            Text("Document conditions")
                        } footer: {
                            if let message = conditions.validationMessage {
                                Text(message).foregroundStyle(Color.danger)
                            }
                        }
                        .disabled(isSaving || needsReload)
                    }
                }
                if let saveError {
                    Text(saveError).foregroundStyle(Color.danger)
                }
                if needsReload {
                    Button("Reload workflow") { Task { await reloadWorkflow() } }
                        .disabled(isLoading || isSaving)
                }
                if isSaving {
                    ProgressView("Saving trigger…")
                }
            }
            .navigationTitle("Edit trigger")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(isSaving || (isLoading && needsReload))
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
                        .disabled(
                            isLoading || isSaving || needsReload || loadError != nil || !hasChanges
                                || documentConditions?.validationMessage != nil || selectedChoice == nil)
                }
            }
            .task { await load() }
            .onChange(of: selection) {
                if selection == workflow.trigger {
                    documentConditions = initialDocumentConditions
                } else if conditionService != nil, let choice = selectedChoice, choice.canEditDocumentConditions {
                    documentConditions = WorkflowDocumentConditionDraft(choice)
                } else {
                    documentConditions = nil
                }
                if !needsReload { saveError = nil }
            }
        }
        .interactiveDismissDisabled(isSaving || (isLoading && needsReload))
    }

    @ViewBuilder private func conditionField(_ field: WorkflowDocumentConditionDraft.Field) -> some View {
        let value = Binding(
            get: { documentConditions?[field.key] ?? "" },
            set: { documentConditions?[field.key] = $0 }
        )
        let options = field.key.options
        if options.isEmpty {
            LabeledContent(field.title) {
                TextField(
                    field.title, text: value, prompt: Text(field.key.valueType == "date" ? "YYYY-MM-DD" : "Any")
                )
                .multilineTextAlignment(.trailing)
                .autocorrectionDisabled()
            }
        } else {
            Picker(field.title, selection: value) {
                Text("Any").tag("")
                if !value.wrappedValue.isEmpty && !options.contains(where: { $0.id == value.wrappedValue }) {
                    Text("Unavailable value").tag(value.wrappedValue)
                }
                ForEach(options) { option in
                    Text(option.title).tag(option.id)
                }
            }
            .pickerStyle(.menu)
        }
    }

    private func loadDocumentConditions(for configuration: WorkflowConfigurationSummary) async throws
        -> WorkflowDocumentConditionDraft?
    {
        guard let conditionService,
            let choice = choices.first(where: { $0.id == configuration.trigger && $0.canEditDocumentConditions })
        else { return nil }
        return try await conditionService.loadWorkflowDocumentConditions(configuration, choice: choice)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        loadError = nil
        defer { isLoading = false }
        do {
            choices = try await service.workflowTriggerChoices(vaultID: workflow.vaultID)
            initialDocumentConditions = try await loadDocumentConditions(for: workflow)
            documentConditions = initialDocumentConditions
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
            saveError = "The workflow changed. Reload it before editing."
        } catch {
            guard !Task.isCancelled else { return }
            loadError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't load triggers. Try again.")
        }
    }

    private func reloadWorkflow() async {
        guard !isLoading, !isSaving, needsReload else { return }
        isLoading = true
        saveError = nil
        defer { isLoading = false }
        do {
            let latest = try await service.workflowConfiguration(vaultID: workflow.vaultID, workflowID: workflow.id)
            guard latest.revision != nil,
                let current = onUpdated(latest, choices.first(where: { $0.id == latest.trigger }))
            else { throw FolioError.unreadableResponse }
            let conditions = try await loadDocumentConditions(for: current)
            workflow = current
            selection = current.trigger
            initialDocumentConditions = conditions
            documentConditions = conditions
            needsReload = false
        } catch {
            saveError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't reload the workflow. Try again.")
        }
    }

    private func save() async {
        guard !isLoading, !isSaving, !needsReload, loadError == nil, hasChanges,
            documentConditions?.validationMessage == nil, let choice = selectedChoice
        else { return }
        isSaving = true
        saveError = nil
        defer { isSaving = false }
        do {
            let saved: WorkflowConfigurationSummary
            if let conditionService, documentConditions != nil || initialDocumentConditions != nil {
                saved = try await conditionService.saveWorkflowTrigger(
                    workflow, choice: choice, documentConditions: documentConditions
                )
            } else {
                saved = try await service.changeWorkflowTrigger(workflow, to: choice.id)
            }
            _ = onUpdated(saved, choice)
            dismiss()
        } catch FolioError.server(status: 400, message: _), FolioError.server(status: 422, message: _) {
            saveError = "Choose a trigger compatible with the workflow's conditions and actions."
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
            saveError = "The workflow changed. Reload it before editing."
        } catch {
            saveError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't save the trigger. Try again.")
        }
    }
}
