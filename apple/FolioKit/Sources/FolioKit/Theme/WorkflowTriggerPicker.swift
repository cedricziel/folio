import SwiftUI

struct WorkflowTriggerPicker: View {
    private let title: String
    private let choices: [WorkflowTriggerChoice]
    @Binding private var selection: String

    init(_ title: String = "When", choices: [WorkflowTriggerChoice], selection: Binding<String>) {
        self.title = title
        self.choices = choices
        _selection = selection
    }

    var body: some View {
        Picker(title, selection: $selection) {
            if !choices.contains(where: { $0.id == selection }) {
                Text(selection.isEmpty ? "Choose a trigger" : "Unavailable trigger").tag(selection)
            }
            ForEach(choices) { choice in
                Text(choice.title).tag(choice.id)
            }
        }
    }
}

struct WorkflowTriggerSelectionRows: View {
    let primaryTitle: String
    let primaryIdentifier: String
    let choices: [WorkflowTriggerChoice]
    @Binding var selections: [String]

    var body: some View {
        WorkflowTriggerPicker(primaryTitle, choices: availableChoices(at: 0), selection: selection(at: 0))
            .accessibilityIdentifier(primaryIdentifier)
        ForEach(selections.indices.dropFirst(), id: \.self) { index in
            additionalTrigger(at: index)
        }
        Button("Add trigger") { selections.append("") }
            .disabled(
                selections.contains("") || selections.count >= choices.count
                    || choices.contains { selections.contains($0.id) && $0.requiresSingleSelection })
    }

    private func availableChoices(at index: Int) -> [WorkflowTriggerChoice] {
        let others = Set(selections.enumerated().filter { $0.offset != index }.map(\.element))
        return choices.filter { !others.contains($0.id) && (selections.count == 1 || !$0.requiresSingleSelection) }
    }

    private func selection(at index: Int) -> Binding<String> {
        Binding {
            selections.indices.contains(index) ? selections[index] : ""
        } set: { value in
            guard selections.indices.contains(index) else { return }
            selections[index] = value
        }
    }

    @ViewBuilder private func additionalTrigger(at index: Int) -> some View {
        WorkflowTriggerPicker(
            "Additional trigger \(index)", choices: availableChoices(at: index), selection: selection(at: index)
        )
        .pickerStyle(.menu)
        .accessibilityIdentifier("workflow-additional-trigger-\(index)")
        HStack {
            Button("Move up", systemImage: "arrow.up") { selections.swapAt(index, index - 1) }
                .disabled(selections[index].isEmpty)
                .accessibilityLabel("Move up, additional trigger \(index)")
            Spacer()
            Button("Remove", systemImage: "trash", role: .destructive) { selections.remove(at: index) }
                .accessibilityLabel("Remove additional trigger \(index)")
        }
        .buttonStyle(.borderless)
    }
}

struct WorkflowTriggerEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var choices: [WorkflowTriggerChoice] = []
    @State private var selections: [String]
    @State private var cadence: WorkflowCadenceDraft
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
        _selections = State(initialValue: workflow.selectedTriggers)
        _cadence = State(initialValue: WorkflowCadenceDraft(workflow.cadence))
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
                    if supportsMultiple {
                        Section {
                            WorkflowTriggerSelectionRows(
                                primaryTitle: "When", primaryIdentifier: "workflow-primary-trigger",
                                choices: choices, selections: $selections)
                        }
                        .disabled(isSaving || needsReload)
                    } else {
                        WorkflowTriggerPicker(
                            choices: choices,
                            selection: Binding(
                                get: { primaryTrigger }, set: { selections = [$0] })
                        )
                        .pickerStyle(.inline)
                        .disabled(isSaving || needsReload)
                    }
                    if service is any WorkflowAuthoringService {
                        WorkflowScheduleFields(triggerID: primaryTrigger, cadence: $cadence)
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
                    Button("Save") { Task { await save() } }.disabled(!canSave)
                }
            }
            .task { await load() }
        }
        .interactiveDismissDisabled(isSaving || (isLoading && needsReload))
    }

    private var primaryTrigger: String { selections.first ?? "" }

    private var supportsMultiple: Bool {
        workflow.triggers != nil && service is any WorkflowAuthoringService
    }

    private var canSave: Bool {
        guard !isLoading, !isSaving, !needsReload, WorkflowTriggerChoice.matching(selections, in: choices) != nil
        else { return false }
        if service is any WorkflowAuthoringService {
            do {
                let draft = try cadence.payload(for: primaryTrigger)
                return selections != workflow.selectedTriggers || draft != workflow.cadence
            } catch {
                return false
            }
        }
        return primaryTrigger != workflow.trigger
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
            selections = current.selectedTriggers
            cadence = WorkflowCadenceDraft(current.cadence)
            needsReload = false
        } catch {
            saveError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't reload the workflow. Try again.")
        }
    }

    private func save() async {
        guard canSave,
            let choice = choices.first(where: { $0.id == primaryTrigger })
        else { return }
        isSaving = true
        saveError = nil
        defer { isSaving = false }
        do {
            let saved: WorkflowConfigurationSummary
            if let authoring = service as? any WorkflowAuthoringService {
                saved = try await authoring.saveWorkflowTriggers(
                    workflow, to: selections, cadence: cadence.payload(for: choice.id))
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
