import FolioKit
import SwiftUI

struct UploadReviewView: View {
    let files: [UploadFile]
    @State var model: UploadViewModel
    let onFinished: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ForEach(files.indices, id: \.self) { index in
                        Label(files[index].filename, systemImage: "doc")
                    }
                }
                if let destination = model.destination {
                    Section {
                        VaultDestinationPicker(destination: destination)
                            .disabled(model.hasStarted)
                    }
                }
                if case let .failed(message, uploaded) = model.state {
                    Section {
                        Text(message).foregroundStyle(Color.danger)
                        Text("\(uploaded) of \(files.count) uploaded. Retry uploads only the remaining files.")
                    }
                }
            }
            .navigationTitle("Upload documents")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onFinished).disabled(isUploading)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(uploadTitle) {
                        Task {
                            FolioTelemetry.shared.action(.documentUpload)
                            await model.upload(files)
                            if case .finished = model.state {
                                onFinished()
                            }
                        }
                    }
                    .disabled(isUploading || (!model.hasStarted && model.destination?.canUpload == false))
                }
            }
        }
        .interactiveDismissDisabled(isUploading)
        .frame(minWidth: 320, minHeight: 240)
        .telemetryScreen(.uploadReview)
    }

    private var uploadTitle: String {
        switch model.state {
        case .uploading: "Uploading…"
        case .failed: "Retry"
        default: "Upload"
        }
    }

    private var isUploading: Bool {
        if case .uploading = model.state {
            return true
        }
        return false
    }
}
