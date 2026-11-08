import FolioKit
import SwiftUI

/// Files a document: its correspondent, type and tags, each saved as soon as it changes.
struct DocumentClassificationView: View {
    let model: DocumentDetailViewModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                status
                if let options = model.classificationOptions {
                    Section {
                        Picker("From", selection: correspondent) {
                            Text("Not set").tag(UUID?.none)
                            ForEach(choices(options.correspondents.map { ($0.id, $0.name) }, keeping: "From")) {
                                Text($0.name).tag(UUID?.some($0.id))
                            }
                        }
                        Picker("Type", selection: documentType) {
                            Text("Not set").tag(UUID?.none)
                            ForEach(choices(options.documentTypes.map { ($0.id, $0.name) }, keeping: "Type")) {
                                Text($0.name).tag(UUID?.some($0.id))
                            }
                        }
                    }
                    Section("Tags") {
                        if options.tags.isEmpty && model.document.tags.isEmpty {
                            Text("No tags yet").foregroundStyle(Color.inkMuted)
                        }
                        ForEach(tagChoices(options.tags)) { tag in
                            tagRow(tag)
                        }
                    }
                } else if let failure = model.optionsFailure {
                    Section {
                        Text(failure).foregroundStyle(Color.danger)
                        Button("Retry") { Task { await model.loadClassificationOptions() } }
                    }
                } else {
                    ProgressView("Loading classifications…")
                }
            }
            .disabled(model.savingClassification)
            .navigationTitle("Classify")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .disabled(model.savingClassification)
                }
            }
        }
        .task { await model.loadClassificationOptions() }
        #if os(macOS)
            .frame(minWidth: 420, minHeight: 420)
        #endif
    }

    @ViewBuilder
    private var status: some View {
        if model.savingClassification {
            Section { ProgressView("Saving…") }
        } else if let failure = model.classificationFailure {
            Section {
                Text(failure).foregroundStyle(Color.danger)
                if model.canRetryClassification {
                    Button("Retry save") { Task { await model.retryClassification() } }
                }
            }
        }
    }

    private var correspondent: Binding<UUID?> {
        Binding(
            get: { model.document.correspondentID },
            set: { id in Task { await model.classify(.correspondent(id)) } }
        )
    }

    private var documentType: Binding<UUID?> {
        Binding(
            get: { model.document.documentTypeID },
            set: { id in Task { await model.classify(.documentType(id)) } }
        )
    }

    private struct Choice: Identifiable {
        let id: UUID
        let name: String
    }

    /// Keeps the current value selectable when it is no longer active, such as a trashed correspondent.
    /// `field` is the metadata label whose value names it.
    private func choices(_ active: [(UUID, String)], keeping field: String) -> [Choice] {
        var choices = active.map { Choice(id: $0.0, name: $0.1) }
        let currentID = field == "From" ? model.document.correspondentID : model.document.documentTypeID
        if let currentID, !choices.contains(where: { $0.id == currentID }) {
            let name = model.metadata.first { $0.label == field }?.value ?? "Unknown"
            choices.insert(Choice(id: currentID, name: name), at: 0)
        }
        return choices
    }

    private func tagChoices(_ active: [Tag]) -> [Tag] {
        model.document.tags.filter { tag in !active.contains { $0.id == tag.id } } + active
    }

    private func tagRow(_ tag: Tag) -> some View {
        let selected = model.document.tags.contains { $0.id == tag.id }
        return Button {
            Task { await model.classify(selected ? .removeTag(tag.id) : .addTag(tag.id)) }
        } label: {
            HStack {
                Text(tag.displayName).foregroundStyle(Color.ink)
                Spacer()
                if selected {
                    Image(systemName: "checkmark").foregroundStyle(Color.accentColor)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityLabel(tag.displayName)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
