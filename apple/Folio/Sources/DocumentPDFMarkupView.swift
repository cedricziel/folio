import FolioKit
import PaperKit
import SwiftUI

@available(iOS 27.2, macOS 27.2, *)
struct DocumentPDFMarkupView: View {
    let documentID: Document.ID
    let revision: DocumentFileRevision
    let service: any FolioService
    let onPublication: @MainActor () async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var draft: PDFInkDraft?
    @State private var pageIndex = 0
    @State private var failure: String?
    @State private var saving = false
    @State private var conflicted = false
    @State private var pending: UploadFile?
    @State private var requestKey = UUID()

    var body: some View {
        NavigationStack {
            VStack {
                if let draft {
                    if let editor = draft.editor(for: pageIndex) {
                        PaperInkCanvas(editor: editor)
                            .id(pageIndex)
                            .allowsHitTesting(pending == nil && !conflicted && !saving)
                    } else {
                        ContentUnavailableView("Can’t mark up this page", systemImage: "doc")
                    }
                    Stepper(
                        "Page \(pageIndex + 1) of \(draft.pageCount)", value: $pageIndex, in: 0...draft.pageCount - 1
                    )
                    .padding(.horizontal)
                } else if failure == nil {
                    ProgressView("Fetching original…")
                }
                if let failure {
                    Text(failure)
                        .foregroundStyle(Color.danger)
                        .padding()
                }
            }
            .navigationTitle("Mark up and sign")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }.disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(pending == nil ? "Save revision" : "Retry save") { Task { await save() } }
                        .disabled(draft == nil || saving || conflicted)
                }
            }
        }
        .frame(minWidth: 300, minHeight: 430)
        .interactiveDismissDisabled()
        .task { await load() }
    }

    private func load() async {
        do {
            let url = try await service.downloadFile(for: documentID, revision: revision)
            defer { try? FileManager.default.removeItem(at: url) }
            try Task.checkCancellation()
            draft = try PDFInkDraft(data: Data(contentsOf: url))
        } catch {
            if !Task.isCancelled { failure = error.localizedDescription }
        }
    }

    private func save() async {
        guard !saving, !conflicted, let draft else { return }
        saving = true
        failure = nil
        defer { saving = false }
        do {
            let file: UploadFile
            if let pending {
                file = pending
            } else {
                file = UploadFile(
                    data: try draft.export(), filename: revision.filename ?? "Marked-up document.pdf",
                    mimeType: "application/pdf")
                pending = file
            }
            _ = try await service.replaceDocumentFile(
                file, for: documentID, expectedCurrentRevisionID: revision.id, idempotencyKey: requestKey
            )
            await onPublication()
            dismiss()
        } catch FolioError.server(status: 409, message: _) {
            conflicted = true
            failure = "Folio couldn’t save this revision. Close the editor and review File history before trying again."
            await onPublication()
        } catch {
            failure = error.localizedDescription
        }
    }
}

@available(iOS 27.2, macOS 27.2, *)
private struct PaperInkCanvas {
    let editor: PaperMarkupViewController
}

#if os(iOS)
    @available(iOS 27.2, *)
    extension PaperInkCanvas: UIViewControllerRepresentable {
        func makeUIViewController(context: Context) -> PaperMarkupViewController { editor }
        func updateUIViewController(_ controller: PaperMarkupViewController, context: Context) {}
    }
#else
    @available(macOS 27.2, *)
    extension PaperInkCanvas: NSViewControllerRepresentable {
        func makeNSViewController(context: Context) -> PaperMarkupViewController { editor }
        func updateNSViewController(_ controller: PaperMarkupViewController, context: Context) {}
    }
#endif
