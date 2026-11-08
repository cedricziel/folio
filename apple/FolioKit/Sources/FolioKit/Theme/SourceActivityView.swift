import SwiftUI

public struct SourceActivityView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model: SourceActivityViewModel
    private let vaultID: UUID
    private let service: any SourceActivityService
    private let openDocument: (UUID) -> Void

    public init(vaultID: UUID, service: any SourceActivityService, openDocument: @escaping (UUID) -> Void) {
        self.vaultID = vaultID
        self.service = service
        self.openDocument = openDocument
        _model = State(initialValue: SourceActivityViewModel(vaultID: vaultID, service: service))
    }

    public var body: some View {
        NavigationStack {
            List {
                if let error = model.error {
                    Section {
                        Text(error).foregroundStyle(Color.danger)
                        Button("Try again") { Task { await model.load() } }
                    }
                }
                if model.isLoading {
                    ProgressView("Loading sources…")
                }
                if model.hasLoaded && model.sources.isEmpty {
                    ContentUnavailableView(
                        "No ingestion sources", systemImage: "tray",
                        description: Text("Add a folder or mailbox in the web settings to see its activity here."))
                }
                ForEach(model.sources) { source in
                    NavigationLink {
                        SourceActivityDetail(source: source, service: service, openDocument: openDocument)
                    } label: {
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                            Text(source.name).foregroundStyle(Color.ink)
                            Text(source.health.title)
                                .font(FolioTheme.Typography.bodySm)
                                .foregroundStyle(source.health == .unavailable ? Color.danger : Color.inkMuted)
                        }
                    }
                }
            }
            .navigationTitle("Ingestion activity")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
            }
            .refreshable { await model.load() }
            .task { if !model.hasLoaded { await model.load() } }
        }
        .telemetryScreen(.sources)
    }
}

private struct SourceActivityDetail: View {
    let source: SourceActivity
    let openDocument: (UUID) -> Void
    @State private var model: SourceOutcomesViewModel

    init(source: SourceActivity, service: any SourceActivityService, openDocument: @escaping (UUID) -> Void) {
        self.source = source
        self.openDocument = openDocument
        _model = State(
            initialValue: SourceOutcomesViewModel(vaultID: source.vaultID, sourceID: source.id, service: service))
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space4) {
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                    Text(source.health.title)
                        .foregroundStyle(source.health == .unavailable ? Color.danger : Color.ink)
                    if let checked = source.lastCheckedAt {
                        Text("Last checked \(checked.formatted(date: .abbreviated, time: .shortened))")
                            .font(FolioTheme.Typography.bodySm)
                            .foregroundStyle(Color.inkMuted)
                    }
                }
                SourceOutcomesView(model: model, openDocument: openDocument)
            }
            .padding(FolioTheme.Spacing.space4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(source.name)
        .refreshable { await model.load() }
    }
}
