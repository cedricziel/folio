import SwiftUI

struct WorkflowCreationEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @FocusState private var nameFocused: Bool
    @State private var choices: [WorkflowTriggerChoice]?
    @State private var triggerIDs = [""]
    @State private var draft = WorkflowActionDraft()
    @State private var cadence = WorkflowCadenceDraft()
    @State private var loadError: String?
    @State private var saveError: String?
    @State private var isSaving = false
    let vaultID: UUID
    let service: any WorkflowAuthoringService

    var body: some View {
        NavigationStack {
            Form {
                if let choices {
                    Section {
                        TextField("Name", text: $name)
                            .focused($nameFocused)
                            .submitLabel(.done)
                            .onSubmit { nameFocused = false }
                        WorkflowTriggerSelectionRows(
                            primaryTitle: "Trigger", primaryIdentifier: "workflow-new-trigger",
                            choices: choices, selections: $triggerIDs
                        )
                        .onChange(of: triggerIDs) { nameFocused = false }
                    } header: {
                        Text("Workflow")
                    } footer: {
                        Text("New workflows are off until you enable them.")
                    }
                    .disabled(isSaving)
                    if let choice {
                        WorkflowScheduleFields(triggerID: choice.id, cadence: $cadence).disabled(isSaving)
                        WorkflowActionRows(
                            draft: $draft, target: choice.target, vaultID: vaultID,
                            taxonomy: service as? any WorkflowActionTaxonomyService
                        )
                        .disabled(isSaving)
                        if let validation = draft.validationMessage(for: choice.target) {
                            Text(validation).foregroundStyle(Color.danger)
                        }
                    }
                    if let saveError {
                        Text(saveError).foregroundStyle(Color.danger)
                    }
                } else if let loadError {
                    Text(loadError).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load() } }
                } else {
                    ProgressView("Loading triggers…")
                }
            }
            .navigationTitle("New workflow")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") { Task { await create() } }.disabled(!canCreate || isSaving)
                }
            }
            .task {
                if choices == nil { await load() }
            }
        }
        .interactiveDismissDisabled(isSaving)
    }

    private var choice: WorkflowTriggerChoice? {
        choices.flatMap { WorkflowTriggerChoice.matching(triggerIDs, in: $0) }
    }

    private var canCreate: Bool {
        guard let choice, (try? validatedWorkflowName(name)) != nil, cadence.isValid(for: choice.id)
        else { return false }
        return draft.validationMessage(for: choice.target) == nil
    }

    private func load() async {
        loadError = nil
        do {
            choices = try await service.workflowTriggerChoices(vaultID: vaultID)
        } catch {
            guard !Task.isCancelled else { return }
            loadError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't load triggers. Try again.")
        }
    }

    private func create() async {
        guard canCreate, let choice, let choices, !isSaving else { return }
        let selected = triggerIDs.compactMap { id in choices.first { $0.id == id } }
        isSaving = true
        saveError = nil
        defer { isSaving = false }
        do {
            _ = try await service.createWorkflow(
                vaultID: vaultID, name: name, choices: selected, actions: draft,
                cadence: cadence.payload(for: choice.id))
            dismiss()
        } catch {
            saveError = WorkflowLifecycleViewModel.message(
                error,
                fallback: "Couldn't create the workflow. Check the configuration and refresh the list before retrying.")
        }
    }
}

struct WorkflowSettingsEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var enabled: Bool
    @State private var isSaving = false
    @State private var error: String?
    @State private var needsReload = false
    let workflow: WorkflowConfigurationSummary
    let service: any WorkflowAuthoringService
    let onReload: () async throws -> Void
    let onSaved: (WorkflowConfigurationSummary) -> Void

    init(
        workflow: WorkflowConfigurationSummary, service: any WorkflowAuthoringService,
        onReload: @escaping () async throws -> Void,
        onSaved: @escaping (WorkflowConfigurationSummary) -> Void
    ) {
        self.workflow = workflow
        self.service = service
        self.onReload = onReload
        self.onSaved = onSaved
        _name = State(initialValue: workflow.name)
        _enabled = State(initialValue: workflow.enabled)
    }

    var body: some View {
        NavigationStack {
            Form {
                if needsReload { WorkflowChangedNotice(reload: onReload) }
                Section {
                    TextField("Name", text: $name)
                    Toggle("Enabled", isOn: $enabled)
                } footer: {
                    if workflow.enabled && !enabled {
                        Text("Turning off this workflow stops pending runs.")
                    }
                }
                .disabled(isSaving || needsReload)
                if let error {
                    Text(error).foregroundStyle(Color.danger)
                }
            }
            .navigationTitle("Edit workflow settings")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }.disabled(!canSave || isSaving)
                }
            }
        }
        .interactiveDismissDisabled(isSaving)
    }

    private var canSave: Bool {
        guard !needsReload, let name = try? validatedWorkflowName(name) else { return false }
        return name != workflow.name || enabled != workflow.enabled
    }

    private func save() async {
        guard canSave, !isSaving else { return }
        isSaving = true
        error = nil
        defer { isSaving = false }
        do {
            let updated = try await service.saveWorkflowSettings(workflow, name: name, enabled: enabled)
            onSaved(updated)
            dismiss()
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
        } catch {
            self.error = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't save settings. Try again.")
        }
    }
}

struct WorkflowChangedNotice: View {
    @Environment(\.dismiss) private var dismiss
    @State private var isReloading = false
    @State private var error: String?
    let reload: () async throws -> Void

    var body: some View {
        Text("The workflow changed. Reload it before editing.")
            .foregroundStyle(Color.danger)
        if let error { Text(error).foregroundStyle(Color.danger) }
        Button("Reload workflow") {
            Task {
                guard !isReloading else { return }
                isReloading = true
                defer { isReloading = false }
                error = nil
                do {
                    try await reload()
                    dismiss()
                } catch {
                    self.error = WorkflowLifecycleViewModel.message(
                        error, fallback: "Couldn't reload the workflow. Try again.")
                }
            }
        }
        .disabled(isReloading)
    }
}
