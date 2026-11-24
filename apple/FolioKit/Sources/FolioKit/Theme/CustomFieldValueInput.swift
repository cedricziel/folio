import SwiftUI

public struct CustomFieldValueInput: View {
    private let field: CustomFieldDefinition
    @Binding private var draft: CustomFieldValueDraft
    private let metadata: any CustomMetadataService
    private let documents: any FolioService
    @State private var options: [CustomFieldOption] = []
    @State private var cursor: UUID?
    @State private var loadedOptions = false
    @State private var matches: [Document] = []
    @State private var query = ""
    @State private var error: String?
    @State private var loadingOptions = false

    public init(
        field: CustomFieldDefinition, draft: Binding<CustomFieldValueDraft>, metadata: any CustomMetadataService,
        documents: any FolioService
    ) {
        self.field = field
        _draft = draft
        self.metadata = metadata
        self.documents = documents
    }

    public var body: some View {
        Group {
            switch field.draft.kind {
            case .boolean:
                Picker("Value", selection: $draft.boolean) {
                    Text("Choose Yes or No").tag(Bool?.none)
                    Text("Yes").tag(Optional(true))
                    Text("No").tag(Optional(false))
                }
            case .longText:
                TextField("Value", text: $draft.text, axis: .vertical)
            case .money:
                TextField("Amount", text: $draft.text)
                    .accessibilityLabel("Amount")
                    .accessibilityIdentifier("custom-field-amount")
                TextField("Currency code", text: $draft.currency)
                    .accessibilityLabel("Currency code")
                    .accessibilityIdentifier("custom-field-currency")
            case .date:
                if let date = draft.date {
                    DatePicker(
                        "Date",
                        selection: Binding(get: { draft.date ?? date }, set: { draft.date = $0 }),
                        displayedComponents: .date
                    )
                    .environment(\.calendar, CustomFieldValueDraft.calendar)
                    .environment(\.timeZone, CustomFieldValueDraft.calendar.timeZone)
                } else {
                    Button("Set date") { draft.date = .now }
                }
            case .select:
                optionPicker
            case .documentLink:
                documentPicker
            case .text, .integer, .decimal, .url:
                TextField("Value", text: $draft.text)
            }
        }
    }

    private var optionPicker: some View {
        Group {
            Picker("Value", selection: $draft.reference) {
                Text("Choose an option").tag(UUID?.none)
                if let reference = draft.reference, !options.contains(where: { $0.id == reference }) {
                    Text("Saved option").tag(Optional(reference))
                }
                ForEach(options) { option in
                    Text(option.isRetired ? "\(option.label) (retired)" : option.label)
                        .tag(Optional(option.id))
                        .disabled(option.isRetired)
                }
            }
            .task { await loadOptions() }
            if loadingOptions { ProgressView("Loading options…") }
            if let error {
                Text(error).foregroundStyle(Color.danger)
                Button("Retry options") { Task { await loadOptions() } }
            } else if cursor != nil {
                Button("More options") { Task { await loadOptions() } }
            }
        }
    }

    private var documentPicker: some View {
        Group {
            TextField("Search this vault for a document", text: $query)
                .task(id: query) { await searchDocuments() }
            if let error { Text(error).foregroundStyle(Color.danger) }
            ForEach(matches) { document in
                Button {
                    draft.reference = document.id
                } label: {
                    Label(document.title, systemImage: draft.reference == document.id ? "checkmark.circle.fill" : "doc")
                }
            }
            if let reference = draft.reference, !matches.contains(where: { $0.id == reference }) {
                Text("Saved document selected")
            }
        }
    }

    private func searchDocuments() async {
        matches = []
        error = nil
        let search = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !search.isEmpty else { return }
        do {
            try await Task.sleep(for: .milliseconds(250))
            let results = try await documents.documents(matching: search)
            try Task.checkCancellation()
            matches = Array(results.prefix(50))
            error = nil
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }

    private func loadOptions() async {
        guard !loadingOptions, !loadedOptions || cursor != nil else { return }
        loadingOptions = true
        defer { loadingOptions = false }
        error = nil
        do {
            let page = try await metadata.customOptions(fieldID: field.id, after: cursor)
            try Task.checkCancellation()
            var merged = Dictionary(uniqueKeysWithValues: options.map { ($0.id, $0) })
            for option in page where !option.isRetired || option.id == draft.reference {
                merged[option.id] = option
            }
            if let reference = draft.reference, merged[reference] == nil {
                merged[reference] = try await metadata.customOption(fieldID: field.id, id: reference)
                try Task.checkCancellation()
            }
            options = merged.values.sorted {
                $0.position == $1.position ? $0.id.uuidString < $1.id.uuidString : $0.position < $1.position
            }
            cursor = page.count == 50 ? page.last?.id : nil
            loadedOptions = true
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }
}
