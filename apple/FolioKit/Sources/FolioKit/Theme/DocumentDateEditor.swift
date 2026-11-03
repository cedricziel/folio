import SwiftUI

public struct DocumentDateEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var saving = false
    @State private var failure: String?
    private let saveDate: (DocumentDate?) async throws -> Void

    public init(documentDate: DocumentDate?, save: @escaping (DocumentDate?) async throws -> Void) {
        _text = State(initialValue: documentDate?.rawValue ?? "")
        saveDate = save
    }

    private var isValid: Bool {
        text.isEmpty || DocumentDate(rawValue: text) != nil
    }

    public var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Document date", text: $text, prompt: Text("YYYY-MM-DD"))
                        .autocorrectionDisabled()
                    if !text.isEmpty {
                        Button("Clear date") { text = "" }
                    }
                } footer: {
                    if !isValid {
                        Text("Enter a valid date as YYYY-MM-DD, or clear the field.")
                    } else {
                        Text("The date printed on the document. Changing it keeps the import date.")
                    }
                }
                if let failure {
                    Section("Couldn't save the date") {
                        Text(failure)
                    }
                }
            }
            .disabled(saving)
            .navigationTitle("Document date")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(saving ? "Saving…" : "Save") {
                        Task { await save() }
                    }
                    .disabled(saving || !isValid)
                }
            }
            .interactiveDismissDisabled(saving)
        }
    }

    @MainActor private func save() async {
        guard isValid, !saving else { return }
        saving = true
        failure = nil
        defer { saving = false }
        do {
            try await saveDate(text.isEmpty ? nil : DocumentDate(rawValue: text))
            dismiss()
        } catch {
            failure = error.localizedDescription
        }
    }
}
