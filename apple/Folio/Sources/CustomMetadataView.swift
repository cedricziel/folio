import FolioKit
import SwiftUI

struct CustomMetadataView: View {
    let metadata: any CustomMetadataService
    let documents: any FolioService
    let canEdit: Bool
    let openDocument: (UUID) -> Void
    @State private var model: CustomMetadataViewModel
    @State private var editing: CustomFieldDefinition?
    @Environment(\.dismiss) private var dismiss

    init(
        documentID: UUID, metadata: any CustomMetadataService, documents: any FolioService,
        canEdit: Bool, openDocument: @escaping (UUID) -> Void
    ) {
        self.metadata = metadata
        self.documents = documents
        self.canEdit = canEdit
        self.openDocument = openDocument
        _model = State(initialValue: CustomMetadataViewModel(documentID: documentID, service: metadata))
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.fields) { field in
                    let assignment = model.snapshot.values.first { $0.fieldID == field.id }
                    if !field.isRetired || assignment?.value != nil {
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                            Text(field.draft.name).font(FolioTheme.Typography.body)
                            if field.isRetired {
                                Text("Retired field").foregroundStyle(Color.inkMuted)
                            }
                            CustomValueDisplay(
                                field: field, assignment: assignment, metadata: metadata,
                                openDocument: { id in
                                    dismiss()
                                    openDocument(id)
                                }
                            )
                            if canEdit {
                                Button("Edit \(field.draft.name)") { editing = field }
                            }
                        }
                    }
                }
                if model.isLoading { ProgressView("Loading custom fields…") }
                if let error = model.error {
                    Text(error).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await model.load(more: model.hasMore) } }
                } else if model.hasMore {
                    Button("More fields") { Task { await model.load(more: true) } }
                } else if !model.isLoading && model.fields.isEmpty {
                    Text("This vault has no custom fields.").foregroundStyle(Color.inkMuted)
                }
            }
            .navigationTitle("Custom fields")
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .task { await model.load() }
            .sheet(item: $editing) { field in
                CustomValueEditor(
                    field: field, current: model.snapshot.values.first { $0.fieldID == field.id }?.value,
                    metadata: metadata, documents: documents, model: model
                )
            }
        }
    }
}

private struct CustomValueDisplay: View {
    let field: CustomFieldDefinition
    let assignment: CustomFieldAssignment?
    let metadata: any CustomMetadataService
    let openDocument: (UUID) -> Void
    @State private var option: CustomFieldOption?
    @State private var optionUnavailable = false

    var body: some View {
        switch assignment?.value {
        case let .documentLink(id):
            if let target = assignment?.link {
                if target.isTrashed {
                    Text("\(target.title) (in Trash)").foregroundStyle(Color.inkMuted)
                } else {
                    Button(target.title, systemImage: "link") { openDocument(id) }
                }
            } else {
                Text("Document unavailable")
            }
        case let .url(value):
            if let url = URL(string: value), ["http", "https"].contains(url.scheme ?? "") {
                Link(value, destination: url)
            } else {
                Text(value)
            }
        case let .select(id):
            Text(
                option.map { $0.isRetired ? "\($0.label) (retired)" : $0.label }
                    ?? (optionUnavailable ? "Option unavailable" : "Loading option…")
            )
            .task(id: id) {
                option = nil
                optionUnavailable = false
                let loaded = try? await metadata.customOption(fieldID: field.id, id: id)
                guard !Task.isCancelled else { return }
                option = loaded
                optionUnavailable = loaded == nil
            }
        case let .text(value), let .longText(value):
            Text(value.isEmpty ? "Empty text" : value).textSelection(.enabled)
        case let .integer(value), let .decimal(value), let .date(value):
            Text(value).textSelection(.enabled)
        case let .money(amount, currency):
            Text("\(amount) \(currency)").textSelection(.enabled)
        case let .boolean(value):
            Text(value ? "Yes" : "No")
        case nil:
            Text(field.draft.required && !field.isRetired ? "Required, not set" : "Not set")
                .foregroundStyle(Color.inkMuted)
        }
    }
}

private struct CustomValueEditor: View {
    let field: CustomFieldDefinition
    let current: CustomFieldValue?
    let metadata: any CustomMetadataService
    let documents: any FolioService
    let model: CustomMetadataViewModel
    @State private var text = ""
    @State private var currency = ""
    @State private var boolean: Bool?
    @State private var date: Date?
    @State private var reference: UUID?
    @State private var options: [CustomFieldOption] = []
    @State private var optionsCursor: UUID?
    @State private var loadedOptions = false
    @State private var matches: [FolioKit.Document] = []
    @State private var query = ""
    @State private var error: String?
    @State private var pickerError: String?
    @State private var loadingChoices = false
    @State private var saving: Task<Void, Never>?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                if let error { Text(error).foregroundStyle(Color.danger) }
                if model.needsReload {
                    Button("Reload current value") {
                        Task {
                            do {
                                try await model.reload()
                                error = nil
                            } catch {
                                self.error = error.localizedDescription
                            }
                        }
                    }
                }
                if field.isRetired {
                    Text("Retired field")
                } else {
                    valueControls
                }
                if current != nil && (!field.draft.required || field.isRetired) {
                    Button("Clear value", role: .destructive) { save(nil) }
                        .disabled(model.isSaving || model.needsReload)
                }
            }
            .navigationTitle(field.draft.name)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                if !field.isRetired {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") {
                            if let value { save(value) }
                        }
                        .disabled(value == nil || model.needsReload || model.isSaving)
                    }
                }
            }
            .task {
                restoreDraft()
                if field.draft.kind == .select { await loadOptions() }
            }
            .task(id: query) {
                guard field.draft.kind == .documentLink else { return }
                let search = query.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !search.isEmpty else { matches = []; return }
                do {
                    try await Task.sleep(for: .milliseconds(250))
                    let results = try await documents.documents(matching: search)
                    try Task.checkCancellation()
                    matches = Array(results.prefix(50))
                    pickerError = nil
                } catch is CancellationError {
                } catch {
                    guard !Task.isCancelled else { return }
                    pickerError = error.localizedDescription
                }
            }
            .onDisappear { saving?.cancel() }
        }
    }

    @ViewBuilder private var valueControls: some View {
        switch field.draft.kind {
        case .boolean:
            Picker("Value", selection: $boolean) {
                Text("Choose Yes or No").tag(Bool?.none)
                Text("Yes").tag(Optional(true))
                Text("No").tag(Optional(false))
            }
        case .longText:
            TextField("Value", text: $text, axis: .vertical)
        case .money:
            TextField("Amount", text: $text)
            TextField("Currency code", text: $currency)
        case .date:
            if let date {
                DatePicker(
                    "Date",
                    selection: Binding(get: { self.date ?? date }, set: { self.date = $0 }),
                    displayedComponents: .date
                )
                .environment(\.calendar, Self.dateCalendar)
                .environment(\.timeZone, Self.dateCalendar.timeZone)
            } else {
                Button("Set date") { date = .now }
            }
        case .select:
            Picker("Value", selection: $reference) {
                Text("Choose an option").tag(UUID?.none)
                ForEach(options) { option in
                    Text(option.isRetired ? "\(option.label) (retired)" : option.label)
                        .tag(Optional(option.id))
                        .disabled(option.isRetired)
                }
            }
            if loadingChoices { ProgressView("Loading options…") }
            if let pickerError {
                Text(pickerError).foregroundStyle(Color.danger)
                Button("Retry options") { Task { await loadOptions() } }
            } else if optionsCursor != nil {
                Button("More options") { Task { await loadOptions() } }
            }
        case .documentLink:
            TextField("Search this vault for a document", text: $query)
            if let pickerError { Text(pickerError).foregroundStyle(Color.danger) }
            ForEach(matches) { document in
                Button {
                    reference = document.id
                } label: {
                    Label(document.title, systemImage: reference == document.id ? "checkmark.circle.fill" : "doc")
                }
            }
            if case let .documentLink(id) = current, reference == id {
                Text("Current link selected")
            }
        case .text, .integer, .decimal, .url:
            TextField("Value", text: $text)
        }
    }

    private var value: CustomFieldValue? {
        switch field.draft.kind {
        case .text: .text(text)
        case .longText: .longText(text)
        case .boolean: boolean.map(CustomFieldValue.boolean)
        case .integer: .integer(text)
        case .decimal: .decimal(text)
        case .date: date.map(Self.calendarDate)
        case .money: .money(amount: text, currency: currency)
        case .select: reference.map(CustomFieldValue.select)
        case .url: .url(text)
        case .documentLink: reference.map(CustomFieldValue.documentLink)
        }
    }

    private static let dateCalendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        return calendar
    }()

    private static func calendarDate(_ date: Date) -> CustomFieldValue {
        .date(date.formatted(.iso8601.year().month().day().dateSeparator(.dash)))
    }

    private func restoreDraft() {
        switch current {
        case let .text(value), let .longText(value), let .integer(value), let .decimal(value), let .url(value):
            text = value
        case let .date(value):
            date = DocumentDate(rawValue: value)?.start(in: Self.dateCalendar.timeZone)
        case let .boolean(value): boolean = value
        case let .money(amount, code):
            text = amount
            currency = code
        case let .select(id), let .documentLink(id): reference = id
        case nil: break
        }
    }

    private func loadOptions() async {
        guard !loadingChoices, !loadedOptions || optionsCursor != nil else { return }
        loadingChoices = true
        defer { loadingChoices = false }
        pickerError = nil
        do {
            let next = try await metadata.customOptions(fieldID: field.id, after: optionsCursor)
            try Task.checkCancellation()
            var merged = Dictionary(uniqueKeysWithValues: options.map { ($0.id, $0) })
            for option in next where !option.isRetired || option.id == reference {
                merged[option.id] = option
            }
            if let reference, merged[reference] == nil {
                merged[reference] = try await metadata.customOption(fieldID: field.id, id: reference)
                try Task.checkCancellation()
            }
            options = merged.values.sorted {
                $0.position == $1.position ? $0.id.uuidString < $1.id.uuidString : $0.position < $1.position
            }
            optionsCursor = next.count == 50 ? next.last?.id : nil
            loadedOptions = true
        } catch is CancellationError {
        } catch {
            guard !Task.isCancelled else { return }
            pickerError = error.localizedDescription
        }
    }

    private func save(_ value: CustomFieldValue?) {
        guard !model.isSaving else { return }
        error = nil
        saving = Task {
            do {
                try await model.save(fieldID: field.id, value: value)
                dismiss()
            } catch {
                guard !Task.isCancelled else { return }
                self.error = error.localizedDescription
            }
        }
    }
}
