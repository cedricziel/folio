import SwiftUI

public struct WorkflowLifecycleView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model: WorkflowLifecycleViewModel
    private let service: any WorkflowLifecycleService

    public init(vaultID: UUID, service: any WorkflowLifecycleService, canManage: Bool) {
        self.service = service
        _model = State(
            initialValue: WorkflowLifecycleViewModel(vaultID: vaultID, service: service, canManage: canManage)
        )
    }

    public var body: some View {
        NavigationStack {
            List {
                if model.canManage {
                    Picker(
                        "Show workflows",
                        selection: Binding(
                            get: { model.deleted },
                            set: { deleted in Task { await model.load(deleted: deleted) } }
                        )
                    ) {
                        Text("Active").tag(false)
                        Text("Trash").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .disabled(model.isMutating)
                    if let error = model.error {
                        Text(error).foregroundStyle(Color.danger)
                        Button("Try again") { Task { await model.load() } }
                    }
                    if model.isLoading {
                        ProgressView("Loading workflows…")
                    }
                    if model.hasLoaded && model.workflows.isEmpty && model.error == nil {
                        ContentUnavailableView(
                            model.deleted ? "No workflows in Trash" : "No workflows",
                            systemImage: model.deleted ? "trash" : "arrow.triangle.branch",
                            description: Text(
                                model.deleted
                                    ? "Removed configurations and their run history appear here."
                                    : "Create a workflow in the web settings to see its configuration and run history here."
                            )
                        )
                    }
                    ForEach(model.workflows) { workflow in
                        NavigationLink {
                            WorkflowLifecycleDetail(workflow: workflow, lifecycle: model, service: service)
                        } label: {
                            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                                Text(workflow.name).foregroundStyle(Color.ink)
                                Text(workflow.stateTitle).foregroundStyle(Color.inkMuted)
                                Text(model.triggerTitle(for: workflow)).foregroundStyle(Color.inkMuted)
                                    .font(FolioTheme.Typography.bodySm)
                                Text(workflow.deletedAt ?? workflow.updatedAt, format: .dateTime.year().month().day())
                                    .font(FolioTheme.Typography.meta).foregroundStyle(Color.inkMuted)
                            }
                        }
                    }
                } else {
                    ContentUnavailableView(
                        "Owner access required", systemImage: "lock",
                        description: Text("Only vault owners can manage workflows.")
                    )
                }
            }
            .navigationTitle("Workflows")
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } } }
            .refreshable { await model.load() }
            .task {
                if !model.hasLoaded {
                    await model.load()
                }
            }
        }
    }
}

private struct WorkflowLifecycleDetail: View {
    @Environment(\.dismiss) private var dismiss
    @State private var workflow: WorkflowConfigurationSummary
    let lifecycle: WorkflowLifecycleViewModel
    private let editingService: (any WorkflowTriggerEditingService)?
    @State private var showingTriggerEditor = false
    @State private var history: WorkflowHistoryViewModel
    @State private var confirmingTrash = false

    init(
        workflow: WorkflowConfigurationSummary, lifecycle: WorkflowLifecycleViewModel,
        service: any WorkflowLifecycleService
    ) {
        _workflow = State(initialValue: workflow)
        self.lifecycle = lifecycle
        editingService = service as? any WorkflowTriggerEditingService
        _history = State(
            initialValue: WorkflowHistoryViewModel(
                vaultID: workflow.vaultID, workflowID: workflow.id, service: service, canManage: lifecycle.canManage
            )
        )
    }

    var body: some View {
        List {
            if lifecycle.accessDenied || history.accessDenied {
                ContentUnavailableView(
                    "Owner access required", systemImage: "lock",
                    description: Text("Only vault owners can manage workflows.")
                )
                Button("Try again") {
                    Task {
                        await lifecycle.load()
                        await history.load()
                    }
                }
            } else {
                Section("Configuration") {
                    LabeledContent("State", value: workflow.stateTitle)
                    Text(lifecycle.triggerTitle(for: workflow))
                    if editingService != nil && workflow.deletedAt == nil {
                        Button("Edit trigger") { showingTriggerEditor = true }
                            .disabled(lifecycle.isMutating || !lifecycle.canManage)
                    }
                    LabeledContent("Created") {
                        Text(workflow.createdAt, format: .dateTime.year().month().day().hour().minute())
                    }
                    LabeledContent("Updated") {
                        Text(workflow.updatedAt, format: .dateTime.year().month().day().hour().minute())
                    }
                    LabeledContent("Created by", value: workflow.createdBy ?? "System or unknown")
                    LabeledContent("Updated by", value: workflow.updatedBy ?? "System or unknown")
                    if let deletedAt = workflow.deletedAt {
                        LabeledContent("Moved to Trash") {
                            Text(deletedAt, format: .dateTime.year().month().day().hour().minute())
                        }
                        Text(
                            "Restoring keeps run history. Enabled workflows resume for future events. Stopped runs stay stopped."
                        )
                        .font(FolioTheme.Typography.bodySm).foregroundStyle(Color.inkMuted)
                    }
                }
                Section("Recent runs") {
                    if let error = history.error {
                        Text(error).foregroundStyle(Color.danger)
                        Button("Try again") { Task { await history.load() } }
                    }
                    if history.isLoading {
                        ProgressView("Loading run history…")
                    }
                    if history.hasLoaded && history.runs.isEmpty && history.error == nil {
                        Text("No runs recorded.").foregroundStyle(Color.inkMuted)
                    }
                    ForEach(history.runs) { run in
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                            Text(run.statusTitle).foregroundStyle(run.status == "failed" ? Color.danger : Color.ink)
                            Text(run.createdAt, format: .dateTime.year().month().day().hour().minute())
                                .font(FolioTheme.Typography.meta).foregroundStyle(Color.inkMuted)
                            Text("Attempts: \(run.attempts). Completed actions: \(run.completedActions).")
                                .font(FolioTheme.Typography.bodySm)
                            if let status = run.responseStatus {
                                Text("HTTP status: \(status)").font(FolioTheme.Typography.meta)
                            }
                            if let message = run.lastError {
                                Text(message).foregroundStyle(Color.danger).textSelection(.enabled)
                            }
                            if let finished = run.finishedAt {
                                Text("Finished \(finished.formatted(date: .abbreviated, time: .shortened))")
                                    .font(FolioTheme.Typography.bodySm).foregroundStyle(Color.inkMuted)
                            }
                        }
                    }
                    Text("Shows up to 50 recent runs, including history retained in Trash.")
                        .font(FolioTheme.Typography.bodySm).foregroundStyle(Color.inkMuted)
                }
                Section {
                    if let error = lifecycle.mutationError {
                        Text(error).foregroundStyle(Color.danger)
                    }
                    if lifecycle.isMutating {
                        ProgressView("Saving workflow…")
                    }
                    if workflow.deletedAt != nil {
                        Button(lifecycle.mutationError == nil ? "Restore workflow" : "Try restoring again") {
                            Task {
                                if await lifecycle.restore(workflow) {
                                    dismiss()
                                }
                            }
                        }
                    } else {
                        Button(
                            lifecycle.mutationError == nil ? "Move workflow to Trash" : "Try moving to Trash again",
                            role: .destructive
                        ) {
                            confirmingTrash = true
                        }
                    }
                }
                .disabled(lifecycle.isMutating || !lifecycle.canManage)
            }
        }
        .navigationTitle(workflow.name)
        .refreshable { await history.load() }
        .task {
            lifecycle.dismissMutationError()
            if !history.hasLoaded {
                await history.load()
            }
        }
        .sheet(isPresented: $showingTriggerEditor) {
            if let editingService {
                WorkflowTriggerEditor(workflow: workflow, service: editingService) { saved, choice in
                    workflow = saved
                    lifecycle.applyTriggerUpdate(saved, choice: choice)
                }
            }
        }
        .confirmationDialog("Move workflow to Trash?", isPresented: $confirmingTrash) {
            Button("Move to Trash", role: .destructive) {
                Task {
                    if await lifecycle.moveToTrash(workflow) {
                        dismiss()
                    }
                }
            }
        } message: {
            Text("Pending runs will stop. The configuration and run history will be retained in Trash.")
        }
    }
}
