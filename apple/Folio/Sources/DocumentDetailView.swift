import FolioKit
import SwiftUI

struct DocumentDetailView: View {
    @State var model: DocumentDetailViewModel
    @State private var showingDiscussion = false
    @State private var editingDate = false
    let vaultID: UUID

    init(model: DocumentDetailViewModel, vaultID: UUID) {
        _model = State(initialValue: model)
        self.vaultID = vaultID
    }

    var body: some View {
        VStack(spacing: 0) {
            preview
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.paperSunken)
            metadataPanel
        }
        .background(Color.paper)
        .navigationTitle(model.document.title)
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .sheet(isPresented: $showingDiscussion) { DocumentDiscussionView(model: model.discussion) }
        .sheet(isPresented: $editingDate) {
            DocumentDateEditor(documentDate: model.document.documentDate) { date in
                try await model.saveDocumentDate(date)
            }
        }
        .task { await model.loadPreview() }
        .task { await model.refreshMetadata() }
        .task { await SpotlightDonor.donate(model.summary, vaultID: vaultID) }
    }

    @ViewBuilder
    private var preview: some View {
        switch model.preview {
        case .loading:
            Text("Fetching the file…")
                .font(FolioTheme.Typography.body)
                .foregroundStyle(Color.inkMuted)
        case let .ready(url):
            QuickLookPreview(url: url)
        case let .failed(message):
            ContentUnavailableView {
                Label("Couldn't open this file", systemImage: "exclamationmark.triangle")
                    .foregroundStyle(Color.danger)
            } description: {
                Text(message)
            } actions: {
                Button("Try again") { Task { await model.loadPreview() } }
            }
        }
    }

    private var metadataPanel: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space4) {
                Text(model.document.title)
                    .font(FolioTheme.Typography.title)
                    .foregroundStyle(Color.ink)
                Grid(
                    alignment: .leading, horizontalSpacing: FolioTheme.Spacing.space4,
                    verticalSpacing: FolioTheme.Spacing.space2
                ) {
                    ForEach(model.metadata) { field in
                        GridRow {
                            Text(field.label.uppercased())
                                .font(FolioTheme.Typography.label)
                                .foregroundStyle(Color.inkMuted)
                            Text(field.value)
                                .font(field.isRecord ? FolioTheme.Typography.meta : FolioTheme.Typography.body)
                                .foregroundStyle(Color.ink)
                                .textSelection(.enabled)
                        }
                    }
                }
                DocumentProcessingView(document: model.document)
                Button("Notes and comments", systemImage: "text.bubble") { showingDiscussion = true }
                if let failure = model.metadataFailure {
                    Text(failure).foregroundStyle(Color.danger)
                    Button("Retry details") { Task { await model.refreshMetadata() } }
                }
                if model.document.canEditDocumentDate {
                    Button("Edit document date") { editingDate = true }
                        .font(FolioTheme.Typography.body)
                }
                if !model.document.tags.isEmpty {
                    HStack(spacing: FolioTheme.Spacing.space2) {
                        ForEach(model.document.tags) { TagCapsule(tag: $0) }
                    }
                }
            }
            .padding(FolioTheme.Spacing.space4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxHeight: 220)
        .background(Color.paperRaised)
        .overlay(alignment: .top) { Rectangle().fill(Color.rule).frame(height: 1) }
    }
}
