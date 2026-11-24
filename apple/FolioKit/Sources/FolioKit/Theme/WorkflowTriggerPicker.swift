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
    @State private var isLoading = false
    @State private var isSaving = false
    @State private var loadError: String?
    @State private var saveError: String?
    @State private var needsReload = false
    @State private var workflow: WorkflowConfigurationSummary
    private let service: any WorkflowTriggerEditingService
    private let onUpdated: (WorkflowConfigurationSummary, WorkflowTriggerChoice?) -> WorkflowConfigurationSummary?

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
                            isLoading || isSaving || needsReload || selection == workflow.trigger
                                || !choices.contains(where: { $0.id == selection }))
                }
            }
            .task { await load() }
        }
        .interactiveDismissDisabled(isSaving || (isLoading && needsReload))
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        loadError = nil
        defer { isLoading = false }
        do {
            choices = try await service.workflowTriggerChoices(vaultID: workflow.vaultID)
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
            workflow = current
            selection = current.trigger
            needsReload = false
        } catch {
            saveError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't reload the workflow. Try again.")
        }
    }

    private func save() async {
        guard !isSaving, !needsReload, selection != workflow.trigger,
            let choice = choices.first(where: { $0.id == selection })
        else { return }
        isSaving = true
        saveError = nil
        defer { isSaving = false }
        do {
            _ = onUpdated(try await service.changeWorkflowTrigger(workflow, to: choice.id), choice)
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
