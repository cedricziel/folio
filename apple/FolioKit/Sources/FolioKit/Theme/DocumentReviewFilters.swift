import SwiftUI

public struct DocumentReviewFilters: View {
    @Environment(\.dismiss) private var dismiss
    @State private var review: Int
    @State private var from: String
    @State private var to: String
    private let original: DocumentFilter
    private let apply: (DocumentFilter) -> Void

    public init(filter: DocumentFilter, apply: @escaping (DocumentFilter) -> Void) {
        original = filter
        self.apply = apply
        _review = State(initialValue: filter.reviewed.map { $0 ? 2 : 1 } ?? 0)
        _from = State(initialValue: filter.documentDateFrom?.rawValue ?? "")
        _to = State(initialValue: filter.documentDateTo?.rawValue ?? "")
    }

    private var filter: DocumentFilter? {
        try? original.applyingReview(review == 0 ? nil : review == 2, from: from, to: to)
    }

    public var body: some View {
        NavigationStack {
            Form {
                Picker("Review", selection: $review) {
                    Text("Any review state").tag(0)
                    Text("Not reviewed").tag(1)
                    Text("Reviewed").tag(2)
                }
                Section {
                    TextField("Document date from", text: $from, prompt: Text("YYYY-MM-DD"))
                        .autocorrectionDisabled()
                    TextField("Document date to", text: $to, prompt: Text("YYYY-MM-DD"))
                        .autocorrectionDisabled()
                } footer: {
                    Text(
                        filter == nil
                            ? "Enter valid dates with the end on or after the start."
                            : "Both dates are included. Undated documents do not match a date range.")
                }
                Button("Clear review and date filters") {
                    review = 0; from = ""; to = ""
                }
            }
            .navigationTitle("Document filters")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Apply") {
                        if let filter { apply(filter); dismiss() }
                    }.disabled(filter == nil)
                }
            }
        }
    }
}
