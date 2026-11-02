import FolioKit
import SwiftUI

/// "Review a scan": the captured pages and a primary "File it" button.
/// AI suggestions (From, Type, Date, Amount, tags) are not part of the API yet.
struct ScanReviewView: View {
    @State var model: ScanReviewViewModel
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
                                Image(decorative: model.pages[index], scale: 1)
                                    .resizable()
                                    .aspectRatio(contentMode: .fit)
                                    .frame(height: 320)
                                    .clipShape(.rect(cornerRadius: FolioTheme.Radius.radiusSm))
                                    .shadow(color: .black.opacity(0.1), radius: 8, y: 3)
                                    .accessibilityLabel("Page \(index + 1)")
                            }
                        }
                        .padding(.vertical, FolioTheme.Spacing.space2)
                    }
                    .scrollIndicators(.hidden)

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
                    Button("Discard") { onFinished(false) }
                }
            }
            .safeAreaInset(edge: .bottom) {
                Button {
                    Task {
                        await model.fileIt()
                        if model.state == .filed {
                            onFinished(true)
                        }
                    }
                } label: {
                    Text(model.state == .filing ? "Filing…" : "File it").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(model.state == .filing || model.destination?.canUpload == false)
                .padding(FolioTheme.Spacing.space4)
            }
        }
    }
}
