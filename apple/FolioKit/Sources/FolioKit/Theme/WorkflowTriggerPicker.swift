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
    private let workflow: WorkflowConfigurationSummary
    private let service: any WorkflowTriggerEditingService
    private let onSaved: (WorkflowConfigurationSummary, WorkflowTriggerChoice) -> Void

    init(
        workflow: WorkflowConfigurationSummary, service: any WorkflowTriggerEditingService,
        onSaved: @escaping (WorkflowConfigurationSummary, WorkflowTriggerChoice) -> Void
    ) {
        self.workflow = workflow
        self.service = service
        self.onSaved = onSaved
        _selection = State(initialValue: workflow.trigger)
    }

    var body: some View {
        NavigationStack {
            Form {
                if isLoading {
                    ProgressView("Loading triggers…")
                } else if let loadError {
                    Text(loadError).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load() } }
                } else if choices.isEmpty {
                    Text("No triggers available.").foregroundStyle(Color.inkMuted)
                } else {
                    WorkflowTriggerPicker(choices: choices, selection: $selection)
                        .pickerStyle(.inline)
                        .disabled(isSaving)
                }
                if let saveError {
                    Text(saveError).foregroundStyle(Color.danger)
                }
                if isSaving {
                    ProgressView("Saving trigger…")
                }
            }
            .navigationTitle("Edit trigger")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
                        .disabled(
                            isLoading || isSaving || selection == workflow.trigger
                                || !choices.contains(where: { $0.id == selection }))
                }
            }
            .task { await load() }
        }
        .interactiveDismissDisabled(isSaving)
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

    private func save() async {
        guard !isSaving, selection != workflow.trigger, let choice = choices.first(where: { $0.id == selection })
        else { return }
        isSaving = true
        saveError = nil
        defer { isSaving = false }
        do {
            onSaved(try await service.changeWorkflowTrigger(workflow, to: choice.id), choice)
            dismiss()
        } catch FolioError.server(status: 400, message: _), FolioError.server(status: 422, message: _) {
            saveError = "Choose a trigger compatible with the workflow's conditions and actions."
        } catch FolioError.server(status: 409, message: _) {
            saveError = "The workflow changed. Reopen it before editing."
        } catch {
            saveError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't save the trigger. Try again.")
        }
    }
}
