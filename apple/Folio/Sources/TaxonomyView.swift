import FolioKit
import SwiftUI

struct TaxonomyView: View {
    let service: any FolioService
    let canManage: Bool
    @State private var kind: TaxonomyKind = .tags

    var body: some View {
        VStack(spacing: FolioTheme.Spacing.space3) {
            Picker("Classification", selection: $kind) {
                ForEach(TaxonomyKind.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, FolioTheme.Spacing.space4)
            TaxonomyList(service: service, kind: kind, canManage: canManage)
                .id(kind)
        }
        .navigationTitle("Classifications")
        .background(Color.paper)
    }
}

private struct TaxonomyList: View {
    let canManage: Bool
    @State private var model: TaxonomyViewModel
    @State private var name = ""
    @State private var editing: TaxonomyEntry?
    @State private var deleting: TaxonomyEntry?
    @State private var showEditor = false

    init(service: any FolioService, kind: TaxonomyKind, canManage: Bool) {
        self.canManage = canManage
        _model = State(initialValue: TaxonomyViewModel(service: service, kind: kind))
    }

    var body: some View {
        List {
            if let error = model.error {
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                    Text(error).foregroundStyle(Color.ink).accessibilityAddTraits(.isStaticText)
                    Button("Retry") { Task { await model.load() } }
                }
            }
            Section("Active") {
                ForEach(model.entries) { entry in
                    HStack(alignment: .top, spacing: FolioTheme.Spacing.space3) {
                        details(entry)
                        Spacer()
                        if canManage {
                            Button {
                                editing = entry; name = entry.name; showEditor = true
                            } label: {
                                Image(systemName: "pencil")
                            }.accessibilityLabel("Rename \(entry.name)")
                            Button(role: .destructive) {
                                deleting = entry
                            } label: {
                                Image(systemName: "trash")
                            }.accessibilityLabel("Move \(entry.name) to Trash")
                        }
                    }
                }
                if model.entries.isEmpty { Text("No classifications yet").foregroundStyle(Color.inkMuted) }
                if canManage {
                    Button("Add \(model.kind.singular)") {
                        editing = nil; name = ""; showEditor = true
                    }
                }
            }
            Section("Trash") {
                ForEach(model.trash) { entry in
                    HStack(alignment: .top, spacing: FolioTheme.Spacing.space3) {
                        details(entry)
                        Spacer()
                        if canManage {
                            Button("Restore") { Task { await model.restore(entry) } }
                        }
                    }
                }
                if model.trash.isEmpty { Text("Trash is empty").foregroundStyle(Color.inkMuted) }
            }
        }
        .buttonStyle(.borderless)
        .disabled(model.isBusy)
        .overlay { if model.isBusy { ProgressView() } }
        .task { await model.load() }
        .refreshable { await model.load() }
        .alert(editing == nil ? "Add classification" : "Rename classification", isPresented: $showEditor) {
            TextField("Name", text: $name)
            Button("Cancel", role: .cancel) {}
            Button("Save") { Task { await model.save(name: name, id: editing?.id) } }
                .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .confirmationDialog(
            "Move to Trash?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })
        ) {
            Button("Move to Trash", role: .destructive) {
                if let entry = deleting { Task { await model.delete(entry) } }
                deleting = nil
            }
        } message: {
            Text("Documents keep this classification. You can restore it from Trash.")
        }
    }

    private func details(_ entry: TaxonomyEntry) -> some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
            Text(entry.name).foregroundStyle(Color.ink)
            if entry.deletedAt != nil {
                Text("Deleted").font(FolioTheme.Typography.bodySm).foregroundStyle(Color.inkMuted)
            }
            DisclosureGroup("History") {
                Text("Created \(entry.createdAt.formatted())")
                Text("Created by \(entry.createdBy ?? "System or unknown")")
                Text("Updated \(entry.updatedAt.formatted())")
                Text("Updated by \(entry.updatedBy ?? "System or unknown")")
            }
            .font(FolioTheme.Typography.bodySm)
            .foregroundStyle(Color.inkMuted)
        }
    }
}
