import CoreTransferable
import FolioKit
import SwiftUI
import UniformTypeIdentifiers

struct DocumentFileHistoryView: View {
    let documentID: Document.ID
    let service: any FolioService
    let canEditFile: Bool
    let onPublication: @MainActor () async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var revisions: [DocumentFileRevision] = []
    @State private var nextBefore: Int64?
    @State private var unavailable = false
    @State private var loading = true
    @State private var loadFailure: String?
    @State private var downloadFailure: String?
    @State private var requestedRevision: DocumentFileRevision?
    @State private var export: RevisionExport?
    @State private var showingExporter = false
    @State private var publicationEnabled = false
    @State private var showingImporter = false
    @State private var readingReplacement = false
    @State private var publishing = false
    @State private var pendingPublication: Publication?
    @State private var publicationFailure: String?
    @State private var needsRefresh = false

    private var editableCurrent: UUID? {
        guard canEditFile, publicationEnabled, !needsRefresh else { return nil }
        return revisions.first(where: { $0.isCurrent && $0.verification == .verified })?.id
    }

    var body: some View {
        NavigationStack {
            List {
                if let loadFailure {
                    Text(loadFailure).foregroundStyle(Color.danger)
                    Button("Try again") { Task { await load(before: nextBefore) } }
                } else if unavailable {
                    Text("This server does not support file history.")
                } else if loading {
                    ProgressView("Loading file history…")
                } else if revisions.isEmpty {
                    Text("No file revisions yet.")
                }

                if let downloadFailure {
                    Text(downloadFailure).foregroundStyle(Color.danger)
                }
                if editableCurrent != nil {
                    Button("Choose a replacement file", systemImage: "arrow.trianglehead.clockwise") {
                        showingImporter = true
                    }
                    .disabled(publishing || readingReplacement || loading)
                }
                if let publicationFailure {
                    Text(publicationFailure).foregroundStyle(Color.danger)
                    if needsRefresh {
                        Button("Refresh file history") { Task { await load(before: nil) } }
                            .disabled(loading || publishing)
                    } else if pendingPublication != nil && !publishing && !readingReplacement {
                        Button("Retry publication") { Task { await publish() } }
                    }
                }
                if readingReplacement { ProgressView("Reading the selected file…") }
                if publishing { ProgressView("Publishing the file…") }
                ForEach(revisions) { revision in
                    revisionRow(revision)
                }
                if let nextBefore, loadFailure == nil {
                    Button("Older revisions") { Task { await load(before: nextBefore) } }
                        .disabled(loading || publishing)
                }
            }
            .scrollContentBackground(.hidden)
            .background(Color.paper)
            .navigationTitle("File history")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }.disabled(publishing || readingReplacement)
                }
            }
        }
        .frame(minWidth: 300, minHeight: 400)
        .interactiveDismissDisabled(publishing || readingReplacement)
        .task { await load(before: nil) }
        .task(id: requestedRevision?.id) {
            guard let requestedRevision else { return }
            await download(requestedRevision)
        }
        .fileExporter(
            isPresented: $showingExporter, item: export, defaultFilename: export?.url.lastPathComponent
        ) { result in
            if case let .failure(error) = result { downloadFailure = error.localizedDescription }
            discardExport()
        } onCancellation: {
            discardExport()
        }
        .fileImporter(isPresented: $showingImporter, allowedContentTypes: [.pdf, .image]) { result in
            switch result {
            case let .success(url):
                Task { await prepareReplacement(url) }
            case let .failure(error):
                publicationFailure = error.localizedDescription
            }
        }
    }

    private func revisionRow(_ revision: DocumentFileRevision) -> some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
            HStack {
                Text("Revision \(revision.number)").font(FolioTheme.Typography.body)
                if revision.isCurrent { Text("Current").foregroundStyle(Color.inkMuted) }
            }
            if let filename = revision.filename { Text(filename) }
            VStack(alignment: .leading) {
                HStack {
                    Text(revision.mimeType)
                    if let size = revision.byteSize { Text(size, format: .byteCount(style: .file)) }
                }
                HStack {
                    Text(revision.createdAt, format: .dateTime.year().month().day().hour().minute())
                    if let actor = revision.createdBy { Text(actor) }
                }
                if let source = revision.restoredFromRevisionID {
                    if let prior = revisions.first(where: { $0.id == source }) {
                        Text("Restored from revision \(prior.number)")
                    } else {
                        Text("Restored from an earlier revision")
                    }
                }
            }
            .font(FolioTheme.Typography.meta)
            .foregroundStyle(Color.inkMuted)
            if revision.verification == .unavailable {
                Text("Original unavailable").foregroundStyle(Color.inkMuted)
            } else {
                if revision.verification == .pending {
                    Text("Verification pending").foregroundStyle(Color.inkMuted)
                } else {
                    Text("Verified original").foregroundStyle(Color.inkMuted)
                }
                if requestedRevision?.id == revision.id {
                    ProgressView("Fetching original…")
                } else {
                    Button("Save original", systemImage: "arrow.down.to.line") {
                        downloadFailure = nil
                        requestedRevision = revision
                    }
                    .buttonStyle(.borderless)
                    .disabled(requestedRevision != nil || showingExporter)
                }
                if let current = editableCurrent, !revision.isCurrent, revision.verification == .verified {
                    Button("Restore revision \(revision.number)", systemImage: "clock.arrow.circlepath") {
                        pendingPublication = Publication(source: .restoration(revision.id), expected: current)
                        Task { await publish() }
                    }
                    .buttonStyle(.borderless)
                    .disabled(publishing || readingReplacement || loading)
                }
            }
        }
    }

    private func load(before: Int64?) async {
        loading = true
        loadFailure = nil
        defer { loading = false }
        do {
            let page = try await service.fileRevisions(for: documentID, before: before)
            try Task.checkCancellation()
            unavailable = page == nil
            if before == nil || unavailable { revisions = [] }
            revisions.append(contentsOf: page?.revisions ?? [])
            nextBefore = page?.nextBefore
            publicationEnabled = page?.publicationEnabled == true
            if before == nil {
                needsRefresh = false
                publicationFailure = nil
            }
        } catch {
            guard !Task.isCancelled else { return }
            if case let FolioError.server(status, _) = error, status == 403 || status == 404 {
                revisions = []
                nextBefore = nil
                publicationEnabled = false
            }
            loadFailure = error.localizedDescription
        }
    }

    private func prepareReplacement(_ url: URL) async {
        guard let expected = editableCurrent, !publishing, !readingReplacement else { return }
        pendingPublication = nil
        publicationFailure = nil
        readingReplacement = true
        let (files, _) = await UploadFile.readDocuments([url])
        readingReplacement = false
        guard let file = files.first else {
            publicationFailure = "Couldn’t read the selected file."
            return
        }
        pendingPublication = Publication(source: .replacement(file), expected: expected)
        await publish()
    }

    private func publish() async {
        guard !publishing, let publication = pendingPublication else { return }
        publishing = true
        needsRefresh = false
        publicationFailure = nil
        defer { publishing = false }
        do {
            switch publication.source {
            case let .replacement(file):
                _ = try await service.replaceDocumentFile(
                    file, for: documentID, expectedCurrentRevisionID: publication.expected,
                    idempotencyKey: publication.key
                )
            case let .restoration(revision):
                _ = try await service.restoreDocumentFile(
                    revision, for: documentID, expectedCurrentRevisionID: publication.expected,
                    idempotencyKey: publication.key
                )
            }
            pendingPublication = nil
            await load(before: nil)
            await onPublication()
        } catch FolioError.server(status: 409, message: _) {
            pendingPublication = nil
            needsRefresh = true
            await load(before: nil)
            if needsRefresh {
                publicationFailure =
                    "The file changed while you were editing. Refresh file history before trying again."
            } else {
                publicationFailure =
                    "The file changed while you were editing. Review the current revision before trying again."
                await onPublication()
            }
        } catch {
            publicationFailure = error.localizedDescription
        }
    }

    private struct Publication {
        enum Source {
            case replacement(UploadFile)
            case restoration(UUID)
        }
        let source: Source
        let expected: UUID
        let key = UUID()
    }

    private func download(_ revision: DocumentFileRevision) async {
        defer { requestedRevision = nil }
        do {
            let url = try await service.downloadFile(for: documentID, revision: revision)
            guard !Task.isCancelled else {
                try? FileManager.default.removeItem(at: url)
                return
            }
            export = RevisionExport(url: url)
            showingExporter = true
        } catch {
            if !Task.isCancelled { downloadFailure = error.localizedDescription }
        }
    }

    private func discardExport() {
        if let export { try? FileManager.default.removeItem(at: export.url) }
        export = nil
    }
}

private struct RevisionExport: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .data) { export in
            SentTransferredFile(export.url)
        }
    }
}
