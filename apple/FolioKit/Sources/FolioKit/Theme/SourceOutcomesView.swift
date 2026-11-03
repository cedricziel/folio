import SwiftUI

public struct SourceOutcomesView: View {
    @Bindable private var model: SourceOutcomesViewModel
    private let openDocument: (UUID) -> Void

    public init(model: SourceOutcomesViewModel, openDocument: @escaping (UUID) -> Void) {
        self.model = model
        self.openDocument = openDocument
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space3) {
            Text("Recent outcomes").font(FolioTheme.Typography.label)
            ForEach(model.outcomes) { outcome in
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                    HStack {
                        Text(outcome.outcome.title)
                            .foregroundStyle(outcome.outcome == .failed ? Color.danger : Color.ink)
                        Spacer()
                        Text(outcome.occurredAt, style: .relative)
                            .foregroundStyle(Color.inkMuted)
                    }
                    Text(outcome.outcome.explanation)
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.inkMuted)
                    if let documentID = outcome.documentID {
                        Button("Open document") { openDocument(documentID) }
                    }
                }
                .accessibilityElement(children: .contain)
            }
            if model.isLoading {
                ProgressView("Loading outcomes…")
            } else if let error = model.error {
                Text(error).foregroundStyle(Color.danger)
                Button("Try again") {
                    Task { await model.retry() }
                }
            } else if model.hasLoaded && model.outcomes.isEmpty {
                Text("No item outcomes yet. Outcomes appear after this source handles an item.")
                    .foregroundStyle(Color.inkMuted)
            } else if model.canLoadMore {
                Button("Load earlier outcomes") { Task { await model.loadMore() } }
            }
        }
        .task { if !model.hasLoaded { await model.load() } }
    }
}
