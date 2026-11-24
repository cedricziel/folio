import FolioKit
import SwiftUI

struct ScanReviewView: View {
    private struct TextPage: Identifiable {
        let id: Int
    }

    @State var model: ScanReviewViewModel
    @State private var confirmDiscard = false
    @State private var textPage: TextPage?
    @State private var recognizingText = false
    let onFinished: (Bool) -> Void

    init(model: ScanReviewViewModel, onFinished: @escaping (Bool) -> Void) {
        _model = State(initialValue: model)
        self.onFinished = onFinished
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space6) {
                    ScrollView(.horizontal) {
                        HStack(spacing: FolioTheme.Spacing.space3) {
                            ForEach(model.pages.indices, id: \.self) { index in
                                VStack(spacing: FolioTheme.Spacing.space2) {
                                    Image(decorative: model.pages[index], scale: 1)
                                        .resizable()
                                        .aspectRatio(contentMode: .fit)
                                        .frame(height: 320)
                                        .clipShape(.rect(cornerRadius: FolioTheme.Radius.radiusSm))
                                        .shadow(color: .black.opacity(0.1), radius: 8, y: 3)
                                        .accessibilityLabel("Page \(index + 1)")
                                    Button("Read text") { textPage = TextPage(id: index) }
                                        .accessibilityLabel("Read text on page \(index + 1)")
                                        .disabled(recognizingText || model.state == .filing)
                                }
                            }
                        }
                        .padding(.vertical, FolioTheme.Spacing.space2)
                    }
                    .scrollIndicators(.hidden)

                    if recognizingText, textPage == nil {
                        Text("Stopping text recognition…")
                            .font(FolioTheme.Typography.bodySm)
                            .foregroundStyle(Color.inkMuted)
                    }

                    VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                        Text("TITLE")
                            .font(FolioTheme.Typography.label)
                            .foregroundStyle(Color.inkMuted)
                        TextField("Title", text: $model.title)
                            .font(FolioTheme.Typography.body)
                            .textFieldStyle(.roundedBorder)
                    }

                    if let destination = model.destination {
                        VaultDestinationPicker(destination: destination)
                            .disabled(model.state == .filing)
                    }

                    Text("\(model.pages.count) \(model.pages.count == 1 ? "page" : "pages")")
                        .font(FolioTheme.Typography.meta)
                        .foregroundStyle(Color.inkMuted)

                    if case let .failed(message) = model.state {
                        Label(message, systemImage: "exclamationmark.triangle")
                            .font(FolioTheme.Typography.bodySm)
                            .foregroundStyle(Color.danger)
                    }
                }
                .padding(FolioTheme.Spacing.space4)
            }
            .background(Color.paper)
            .navigationTitle("Review a scan")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Discard") { confirmDiscard = true }
                        .disabled(model.state == .filing)
                }
            }
            .safeAreaInset(edge: .bottom) {
                Button {
                    model.file { onFinished(true) }
                } label: {
                    Text(model.state == .filing ? "Filing…" : "File it").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(model.state == .filing || model.destination?.canUpload == false)
                .padding(FolioTheme.Spacing.space4)
            }
        }
        .interactiveDismissDisabled()
        .onDisappear { model.cancelFiling() }
        .sheet(item: $textPage) { page in
            ScanTextPreview(pageNumber: page.id + 1, image: model.pages[page.id], recognizing: $recognizingText) {
                model.title = $0
                textPage = nil
            }
        }
        .alert("Discard this scan?", isPresented: $confirmDiscard) {
            Button("Discard Scan", role: .destructive) { onFinished(false) }
            Button("Keep Scan", role: .cancel) {}
        } message: {
            Text("These pages haven't been filed. Discarding the scan permanently removes them.")
        }
        .telemetryScreen(.scanReview)
    }
}

private struct ScanTextPreview: View {
    private enum State {
        case reading
        case empty
        case ready
        case failed
    }

    @Environment(\.dismiss) private var dismiss
    @State private var state = State.reading
    @State private var text = ""

    let pageNumber: Int
    let image: CGImage
    @Binding var recognizing: Bool
    let useTitle: (String) -> Void

    var body: some View {
        NavigationStack {
            Group {
                switch state {
                case .ready:
                    VStack(alignment: .leading, spacing: FolioTheme.Spacing.space3) {
                        Text("Edits to this text stay in the preview.")
                            .font(FolioTheme.Typography.bodySm)
                            .foregroundStyle(Color.inkMuted)
                        TextEditor(text: $text)
                            .accessibilityLabel("Recognized text")
                        Button("Use first line as title") { useTitle(firstLine) }
                            .disabled(firstLine.isEmpty)
                    }
                case .failed:
                    Text("Couldn’t read text on this page.")
                case .empty:
                    Text("No text found on this page.")
                case .reading:
                    ProgressView("Reading text…")
                }
            }
            .padding(FolioTheme.Spacing.space4)
            .navigationTitle("Page \(pageNumber) text")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(state == .reading ? "Cancel" : "Done") { dismiss() }
                }
            }
        }
        .task { await readPage() }
    }

    private var firstLine: String {
        String(text.prefix(while: { !$0.isNewline }))
            .trimmingCharacters(in: .whitespaces)
    }

    private func readPage() async {
        guard !recognizing else {
            state = .failed
            return
        }
        recognizing = true
        defer { recognizing = false }
        do {
            let result = try await ScanTextRecognizer.recognize(image)
            try Task.checkCancellation()
            text = result
            state = result.isEmpty ? .empty : .ready
        } catch {
            if !Task.isCancelled { state = .failed }
        }
    }
}
