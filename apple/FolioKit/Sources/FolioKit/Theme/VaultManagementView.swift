import SwiftUI

public struct VaultManagementView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model: VaultManagementViewModel
    @State private var newName = ""
    private let onCatalogChanged: () async -> Void

    public init(service: any VaultManagementService, onCatalogChanged: @escaping () async -> Void = {}) {
        _model = State(initialValue: VaultManagementViewModel(service: service))
        self.onCatalogChanged = onCatalogChanged
    }

    public var body: some View {
        NavigationStack {
            List {
                Section("Create vault") {
                    TextField("Vault name", text: $newName)
                    Button("Create vault", systemImage: "plus") {
                        Task {
                            if await model.create(name: newName) { newName = "" }
                        }
                    }
                    .disabled(newName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.isMutating)
                }
                Section("Your vaults") {
                    ForEach(model.catalog.vaults) { vault in
                        NavigationLink {
                            VaultManagementDetail(model: model, id: vault.id, title: vault.name)
                        } label: {
                            VStack(alignment: .leading) {
                                Text(vault.name)
                                Text(
                                    vault.id == model.catalog.defaultVaultID
                                        ? "\(vault.role.title) · Account default" : vault.role.title
                                )
                                .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    if model.hasLoaded && model.catalog.vaults.isEmpty {
                        Text("No active vaults. Create a vault or restore one below to open your library.")
                            .foregroundStyle(.secondary)
                    }
                }
                if !model.catalog.deletedVaults.isEmpty {
                    Section("Deleted vaults") {
                        ForEach(model.catalog.deletedVaults) { vault in
                            NavigationLink {
                                VaultManagementDetail(model: model, id: vault.id, title: vault.name)
                            } label: {
                                VStack(alignment: .leading) {
                                    Text(vault.name)
                                    if let date = vault.deletedAt {
                                        Text("Deleted \(date.formatted(date: .abbreviated, time: .omitted))")
                                            .font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }
                    }
                }
                if model.isLoading { ProgressView("Loading vaults…") }
            }
            .navigationTitle("Vaults")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh", systemImage: "arrow.clockwise") { Task { await model.load() } }
                        .disabled(model.isLoading || model.isMutating)
                }
            }
            .task { await model.load() }
            .onChange(of: model.catalogRevision) { _, _ in Task { await onCatalogChanged() } }
            .alert(
                "Couldn't update vaults",
                isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.dismissError() } })
            ) {
                Button("OK") { model.dismissError() }
            } message: {
                Text(model.error ?? "")
            }
        }
        .frame(minWidth: 320, idealWidth: 560, minHeight: 400, idealHeight: 640)
    }
}

private struct VaultManagementDetail: View {
    @Bindable var model: VaultManagementViewModel
    let id: UUID
    let title: String
    @State private var name = ""
    @State private var showGrant = false
    @State private var editingMember: VaultMember?
    @State private var revokingMember: VaultMember?
    @State private var confirmDelete = false
    @State private var confirmRestore = false

    private var deleted: Bool { model.catalog.deletedVaults.contains { $0.id == id } }
    private var owner: Bool { model.canManage(id) }
    private var active: Vault? { model.catalog.vaults.first { $0.id == id } }

    var body: some View {
        List {
            if let metadata = model.metadata, metadata.id == id {
                Section("Vault details") {
                    LabeledContent("Name", value: metadata.name)
                    LabeledContent("Created", value: metadata.createdAt.formatted(date: .abbreviated, time: .shortened))
                    if let creator = metadata.createdBy { LabeledContent("Created by", value: creator) }
                    LabeledContent("Updated", value: metadata.updatedAt.formatted(date: .abbreviated, time: .shortened))
                    if let updater = metadata.updatedBy { LabeledContent("Updated by", value: updater) }
                    if let date = metadata.deletedAt {
                        LabeledContent("Deleted", value: date.formatted(date: .abbreviated, time: .shortened))
                    }
                }
            }
            if deleted {
                Section {
                    Text(
                        "The vault and its records are retained. Restoring it does not replay past ingestion or workflow runs."
                    )
                    .foregroundStyle(.secondary)
                    Button("Restore vault", systemImage: "arrow.uturn.backward") { confirmRestore = true }
                }
            } else if let active {
                Section("Account default") {
                    if model.catalog.defaultVaultID == id {
                        Label("Default vault", systemImage: "checkmark")
                        Text("Uploads and links that do not name a vault use this account default.")
                            .font(.caption).foregroundStyle(.secondary)
                    } else {
                        Button("Use as account default") { Task { _ = await model.makeDefault(id: id) } }
                    }
                    if active.role != .owner { Text("\(active.role.title) access").foregroundStyle(.secondary) }
                }
            }
            if owner {
                Section("Rename vault") {
                    TextField("Vault name", text: $name)
                    Button("Save name") { Task { _ = await model.rename(id: id, name: name) } }
                        .disabled(
                            name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || name == model.metadata?.name
                        )
                }
                Section("Members") {
                    ForEach(model.members) { member in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(member.principalID).textSelection(.enabled)
                            Text("\(member.principalType.title) · \(member.role.title)")
                                .font(.caption).foregroundStyle(.secondary)
                            if let grantor = member.grantedBy {
                                Text(
                                    "Granted by \(grantor) on \(member.grantedAt.formatted(date: .abbreviated, time: .omitted))"
                                )
                                .font(.caption).foregroundStyle(.secondary)
                            }
                            HStack {
                                Button("Change role") { editingMember = member }
                                Spacer()
                                Button("Revoke access", role: .destructive) { revokingMember = member }
                            }
                            .buttonStyle(.borderless)
                        }
                    }
                    Button("Grant access", systemImage: "person.badge.plus") { showGrant = true }
                }
                Section {
                    Button("Delete vault", role: .destructive) { confirmDelete = true }
                        .disabled(model.catalog.defaultVaultID == id)
                    if model.catalog.defaultVaultID == id {
                        Text("Choose another account default before deleting this vault.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if owner || deleted {
                Section("History") {
                    ForEach(model.history) { event in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(event.summary)
                            Text(event.at.formatted(date: .abbreviated, time: .shortened))
                                .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                            if let actor = event.actor {
                                Text("By \(actor)").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    if !model.isLoadingDetails && model.history.isEmpty && model.detailError == nil {
                        Text("No history yet.").foregroundStyle(.secondary)
                    }
                }
            }
            if let error = model.detailError {
                Section {
                    Text(error).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await model.loadDetails(id: id) } }
                }
            }
            if model.isLoadingDetails { ProgressView("Loading vault details…") }
            if active == nil && !deleted && !model.isLoading {
                Text("This vault is no longer available to your account.").foregroundStyle(.secondary)
            }
        }
        .disabled(model.isMutating)
        .navigationTitle(model.metadata?.id == id ? model.metadata?.name ?? title : title)
        .task(id: id) {
            await model.loadDetails(id: id)
            name = model.metadata?.name ?? title
        }
        .sheet(isPresented: $showGrant) {
            VaultMemberEditor(model: model, vaultID: id, existing: nil)
        }
        .sheet(item: $editingMember) { member in
            VaultMemberEditor(model: model, vaultID: id, existing: member)
        }
        .confirmationDialog("Delete this vault?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete vault", role: .destructive) { Task { _ = await model.delete(id: id) } }
        } message: {
            Text(
                "The vault will be hidden for everyone. Documents and history are retained for recovery. Ingestion and workflow execution stop until it is restored."
            )
        }
        .confirmationDialog("Restore this vault?", isPresented: $confirmRestore, titleVisibility: .visible) {
            Button("Restore vault") { Task { _ = await model.restore(id: id) } }
                .accessibilityIdentifier("vault-restore-confirmation")
        } message: {
            Text("Previous members regain access. Past ingestion and workflow runs will not be replayed.")
        }
        .confirmationDialog(
            "Revoke access?",
            isPresented: Binding(get: { revokingMember != nil }, set: { if !$0 { revokingMember = nil } }),
            presenting: revokingMember
        ) { member in
            Button("Revoke access", role: .destructive) { Task { _ = await model.revoke(member) } }
        } message: { member in
            Text(
                "Remove the direct grant for \(member.principalID)? Access through another user or group grant may remain."
            )
        }
    }
}

private struct VaultMemberEditor: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var model: VaultManagementViewModel
    let vaultID: UUID
    let existing: VaultMember?
    @State private var principal = ""
    @State private var type = VaultPrincipalType.user
    @State private var role = Vault.Role.viewer
    private let roles: [Vault.Role] = [.viewer, .contributor, .editor, .owner]

    var body: some View {
        NavigationStack {
            Form {
                if let existing {
                    LabeledContent(existing.principalType.title, value: existing.principalID)
                } else {
                    Picker("Principal type", selection: $type) {
                        ForEach(VaultPrincipalType.allCases, id: \.self) { Text($0.title).tag($0) }
                    }
                    TextField("User or group ID", text: $principal)
                        .autocorrectionDisabled()
                    Text("Use the exact user or group ID from your identity provider.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Picker("Role", selection: $role) {
                    ForEach(roles, id: \.self) { Text($0.title).tag($0) }
                }
                Text(
                    "Viewers read documents. Contributors also upload. Editors can edit documents. Owners also manage the vault and access."
                )
                .font(.caption).foregroundStyle(.secondary)
                if let error = model.error { Text(error).foregroundStyle(Color.danger) }
            }
            .navigationTitle(existing == nil ? "Grant access" : "Change role")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(model.isMutating)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task {
                            if await model.grant(
                                id: vaultID, type: existing?.principalType ?? type,
                                principal: existing?.principalID ?? principal, role: role)
                            {
                                dismiss()
                            }
                        }
                    }
                    .disabled(
                        model.isMutating
                            || (existing == nil && principal.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
                }
            }
            .onAppear {
                model.dismissError()
                role = existing.map { roles.contains($0.role) ? $0.role : .viewer } ?? .viewer
            }
        }
        .frame(minWidth: 320, idealWidth: 480, minHeight: 300)
        .interactiveDismissDisabled(model.isMutating)
    }
}
