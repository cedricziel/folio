import FolioAPI
import SwiftUI

struct WorkflowConditionEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var tree: WorkflowConditionTree?
    @State private var choice: WorkflowTriggerChoice?
    @State private var isSaving = false
    @State private var needsReload = false
    @State private var loadError: String?
    @State private var saveError: String?
    let workflow: WorkflowConfigurationSummary
    let service: any WorkflowConditionEditingService
    let onReload: () async throws -> Void
    let onSaved: (WorkflowConfigurationSummary, WorkflowTriggerChoice) -> Void

    var body: some View {
        NavigationStack {
            Form {
                if needsReload { WorkflowChangedNotice(reload: onReload) }
                if let tree = Binding($tree), let choice {
                    let fields = WorkflowConditionTree.Field.available(
                        for: choice, includeLegacyDefaults: workflow.selectedTriggers.count == 1)
                    let supportsActor = WorkflowConditionTree.Actor.isAvailable(for: choice)
                    WorkflowConditionBranch(
                        node: tree, fields: fields, supportsActor: supportsActor, depth: 0,
                        remaining: { WorkflowConditionTree.maximumNodes - tree.wrappedValue.count }
                    )
                    .disabled(isSaving || needsReload)
                    if let message = tree.wrappedValue.validationMessage(fields: fields, supportsActor: supportsActor)
                        ?? saveError
                    {
                        Text(message).foregroundStyle(Color.danger)
                    }
                } else if let loadError {
                    Text(loadError).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load() } }
                } else if !needsReload {
                    ProgressView("Loading conditions…")
                }
            }
            .navigationTitle("Edit conditions")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
                        .disabled(!canSave || isSaving)
                }
            }
            .task { await load() }
        }
        .interactiveDismissDisabled(isSaving)
    }

    private var canSave: Bool {
        guard !needsReload, let tree, let choice else { return false }
        return tree.validationMessage(
            fields: WorkflowConditionTree.Field.available(
                for: choice, includeLegacyDefaults: workflow.selectedTriggers.count == 1),
            supportsActor: WorkflowConditionTree.Actor.isAvailable(for: choice)
        ) == nil
    }

    private func load() async {
        guard tree == nil else { return }
        loadError = nil
        do {
            async let choices = service.workflowTriggerChoices(vaultID: workflow.vaultID)
            async let conditions = service.loadWorkflowConditions(workflow)
            guard let current = WorkflowTriggerChoice.matching(workflow.selectedTriggers, in: try await choices) else {
                throw FolioError.unreadableResponse
            }
            tree = try await conditions
            choice = current
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
        } catch {
            guard !Task.isCancelled else { return }
            loadError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't load conditions. Try again.")
        }
    }

    private func save() async {
        guard canSave, !isSaving, let tree, let choice else { return }
        isSaving = true
        saveError = nil
        defer { isSaving = false }
        do {
            let saved = try await service.saveWorkflowConditions(workflow, conditions: tree)
            onSaved(saved, choice)
            dismiss()
        } catch FolioError.server(status: 409, message: _) {
            needsReload = true
        } catch {
            saveError = WorkflowLifecycleViewModel.message(error, fallback: "Couldn't save the conditions. Try again.")
        }
    }
}

private struct WorkflowConditionBranch: View {
    @Binding var node: WorkflowConditionTree
    let fields: [WorkflowConditionTree.Field]
    let supportsActor: Bool
    let depth: Int
    let remaining: () -> Int

    var body: some View {
        let hasAdditional = node.hasAdditionalFilters(fields: fields, supportsActor: supportsActor)
        if !fields.isEmpty || supportsActor || hasAdditional {
            Section("Filters") {
                ForEach(fields) { field in
                    if field.isSourceKind {
                        WorkflowSourceKindPicker(title: field.title, selection: binding(for: field))
                    } else if field.isTextMatch {
                        WorkflowTextMatchFields(node: $node, field: field)
                    } else if field.processing == .hasText {
                        Picker(field.title, selection: binding(for: field)) {
                            Text("Any").tag("")
                            Text("Yes").tag("true")
                            Text("No").tag("false")
                        }
                    } else if field.processing == .retryTime {
                        TextField(field.title, text: binding(for: field), prompt: Text("2040-01-03T09:30:00Z"))
                            .autocorrectionDisabled()
                    } else {
                        TextField(field.title, text: binding(for: field)).autocorrectionDisabled()
                    }
                }
                if supportsActor {
                    Picker("Actor", selection: $node.actor) {
                        ForEach(WorkflowConditionTree.Actor.allCases) { actor in
                            Text(actor.title).tag(actor)
                        }
                    }
                    if node.actor == .user {
                        TextField("User ID", text: $node.actorUserID).autocorrectionDisabled()
                    }
                }
                if hasAdditional {
                    Text("This condition includes additional saved filters.")
                        .foregroundStyle(Color.inkMuted)
                }
            }
        }
        ForEach(WorkflowConditionTree.Match.allCases) { kind in
            Section(kind.title) {
                if let children = node[kind] {
                    if kind == .any, children.isEmpty {
                        Text("An empty Match any group never matches.").foregroundStyle(Color.inkMuted)
                    }
                    ForEach(Array(children.enumerated()), id: \.element.id) { index, child in
                        NavigationLink("Condition \(index + 1)") {
                            Form {
                                WorkflowConditionBranch(
                                    node: binding(for: child, in: kind), fields: fields,
                                    supportsActor: supportsActor, depth: depth + 1, remaining: remaining
                                )
                            }
                            .navigationTitle("Condition \(index + 1)")
                        }
                    }
                    .onDelete { offsets in node[kind]?.remove(atOffsets: offsets) }
                    Button("Add condition") { node[kind]?.append(.init()) }
                        .disabled(depth >= WorkflowConditionTree.maximumDepth || remaining() <= 0)
                        .accessibilityLabel("Add condition to \(kind.title)")
                    Button("Remove group", role: .destructive) { node[kind] = nil }
                        .accessibilityLabel("Remove \(kind.title) group")
                } else {
                    Button("Add group") { node[kind] = [] }
                        .disabled(depth >= WorkflowConditionTree.maximumDepth)
                        .accessibilityLabel("Add \(kind.title) group")
                }
            }
        }
    }

    private func binding(for field: WorkflowConditionTree.Field) -> Binding<String> {
        Binding(get: { node[field] }, set: { node[field] = $0 })
    }

    private func binding(for child: WorkflowConditionTree, in kind: WorkflowConditionTree.Match)
        -> Binding<WorkflowConditionTree>
    {
        Binding {
            node[kind]?.first(where: { $0.id == child.id }) ?? child
        } set: { updated in
            guard let index = node[kind]?.firstIndex(where: { $0.id == child.id }) else { return }
            node[kind]?[index] = updated
        }
    }
}

private struct WorkflowSourceKindPicker: View {
    let title: String
    @Binding var selection: String

    var body: some View {
        Picker(title, selection: $selection) {
            Text("Any source").tag("")
            if !selection.isEmpty, Components.Schemas.SourceKind(rawValue: selection) == nil {
                Text("Saved source").tag(selection)
            }
            ForEach(Components.Schemas.SourceKind.allCases, id: \.self) { kind in
                Text(label(kind)).tag(kind.rawValue)
            }
        }
    }

    private func label(_ kind: Components.Schemas.SourceKind) -> String {
        switch kind {
        case .upload: "Upload"
        case .api: "API"
        case .folder: "Folder"
        case .email: "Email"
        case .mcp: "MCP"
        case .share: "Share link"
        case ._import: "Import"
        case .paperless: "Paperless-ngx"
        }
    }
}

private struct WorkflowTextMatchFields: View {
    @Binding var node: WorkflowConditionTree
    @State private var mode: WorkflowConditionTree.TextMatch.Mode
    let field: WorkflowConditionTree.Field

    init(node: Binding<WorkflowConditionTree>, field: WorkflowConditionTree.Field) {
        _node = node
        _mode = State(initialValue: node.wrappedValue.textMatch(for: field)?.mode ?? .contains)
        self.field = field
    }

    var body: some View {
        let match = node.textMatch(for: field)
        TextField(
            field.title,
            text: Binding(
                get: { node.textMatch(for: field)?.pattern ?? "" },
                set: { node.setTextMatch($0, mode: mode, for: field) }
            )
        )
        .autocorrectionDisabled()
        Picker("Match \(field.title.lowercased())", selection: $mode) {
            ForEach(WorkflowConditionTree.TextMatch.Mode.allCases) { mode in
                Text(mode.title).tag(mode)
            }
        }
        .onChange(of: mode) {
            if let match {
                node.setTextMatch(match.pattern, mode: mode, for: field)
            }
        }
    }
}
