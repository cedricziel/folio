import FolioKit
import SwiftUI

struct DocumentDiscussionView: View {
    @Bindable var model: DocumentDiscussionViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var deleting: DocumentEntry?

    var body: some View {
        NavigationStack {
            VStack(spacing: FolioTheme.Spacing.space3) {
                Picker(
                    "Entry type",
                    selection: Binding(
                        get: { model.kind },
                        set: { kind in
                            Task { await model.select(kind) }
                        })
                ) {
                    ForEach(DocumentEntryKind.allCases, id: \.self) { kind in
                        Text(kind.title).tag(kind)
                    }
                }
                .pickerStyle(.segmented)
                .disabled(model.isEditing || model.isSaving)
                .padding(.horizontal, FolioTheme.Spacing.space4)

                if let error = model.error {
                    VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                        Label(error, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(Color.danger)
                            .accessibilityAddTraits(.updatesFrequently)
                        if !model.isEditing {
                            Button("Try again") { Task { await model.load() } }
                        }
                    }
                    .padding(.horizontal, FolioTheme.Spacing.space4)
                }

                if model.isEditing {
                    editor
                } else {
                    List {
                        if model.isLoading && model.entries.isEmpty {
                            ProgressView("Loading \(model.kind.title.lowercased())…")
                        } else if model.entries.isEmpty && model.error == nil {
                            Text(
                                model.kind == .note
                                    ? "Add a note to keep useful context with this document."
                                    : "Add a comment to start a discussion about this document."
                            )
                            .foregroundStyle(Color.inkMuted)
                        }
                        ForEach(model.entries) { entry in
                            entryRow(entry)
                        }
                        if model.nextCursor != nil {
                            Button("Load more") { Task { await model.loadMore() } }
                                .disabled(model.isLoading)
                        }
                    }
                    .scrollContentBackground(.hidden)
                    if model.canCreate {
                        Button("Add \(model.kind.singular)", systemImage: "plus") { model.beginAdding() }
                            .buttonStyle(.borderedProminent)
                            .padding(.bottom, FolioTheme.Spacing.space4)
                    }
                }
            }
            .background(Color.paper)
            .navigationTitle("Notes and comments")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .disabled(model.isEditing || model.isSaving)
                }
            }
            .confirmationDialog(
                "Delete this \(deleting?.kind.singular ?? "entry")?",
                isPresented: Binding(
                    get: { deleting != nil }, set: { if !$0 { deleting = nil } }
                ), titleVisibility: .visible, presenting: deleting
            ) { entry in
                Button("Delete", role: .destructive) {
                    Task { await model.delete(entry) }
                    deleting = nil
                }
                .accessibilityIdentifier("confirm-entry-delete")
                Button("Cancel", role: .cancel) { deleting = nil }
            }
        }
        .onChange(of: model.error) { _, message in
            if let message { AccessibilityNotification.Announcement(message).post() }
        }
        .interactiveDismissDisabled(model.isEditing || model.isSaving)
        .task { await model.load() }
        .frame(minWidth: 300, minHeight: 400)
        .telemetryScreen(.documentDiscussion)
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space3) {
            Text(model.editingID == nil ? "New \(model.kind.singular)" : "Edit \(model.kind.singular)")
                .font(FolioTheme.Typography.title)
            TextEditor(text: $model.draft)
                .accessibilityLabel("\(model.kind.singular.capitalized) text")
                .disabled(model.isSaving || model.draftIsLocked)
                .frame(minHeight: 160)
                .overlay(Rectangle().stroke(Color.rule))
            if model.draftIsLocked && !model.isSaving {
                Text("Retry to confirm this text was saved, or cancel to discard this draft.")
                    .font(FolioTheme.Typography.body)
                    .foregroundStyle(Color.inkMuted)
            }
            if model.draft.utf8.count > 16_384 {
                Text("Shorten this text to 16 KB or less.").foregroundStyle(Color.danger)
            }
            HStack {
                Button("Cancel", role: .cancel) { model.cancel() }.disabled(model.isSaving)
                Spacer()
                if model.isSaving { ProgressView() }
                Button(model.draftIsLocked ? "Retry save" : "Save") { Task { await model.save() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(!model.canSave)
            }
        }
        .padding(FolioTheme.Spacing.space4)
    }

    private func entryRow(_ entry: DocumentEntry) -> some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
            Text(entry.body).textSelection(.enabled)
                .font(FolioTheme.Typography.body)
            Text(entry.createdBy ?? "Unknown author")
                .font(FolioTheme.Typography.meta).foregroundStyle(Color.inkMuted)
            HStack {
                Text(entry.createdAt, format: .dateTime.year().month().day().hour().minute())
                if entry.updatedAt > entry.createdAt { Text("Updated") }
            }
            .font(FolioTheme.Typography.meta).foregroundStyle(Color.inkMuted)
            if entry.canEdit || entry.canDelete {
                HStack {
                    if entry.canEdit {
                        Button("Edit") { model.beginEditing(entry) }
                            .accessibilityIdentifier("document-entry-edit")
                    }
                    if entry.canDelete {
                        Button("Delete", role: .destructive) { deleting = entry }
                            .accessibilityIdentifier("document-entry-delete")
                    }
                }
                .buttonStyle(.borderless)
                .disabled(model.isSaving)
            }
        }
        .padding(.vertical, FolioTheme.Spacing.space2)
        .listRowBackground(Color.paperRaised)
    }
}
