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
                            .disabled(isUploading)
                    }
                }
                if case let .failed(message, _) = model.state {
                    Text(message).foregroundStyle(Color.danger)
                }
            }
            .navigationTitle("Upload documents")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onFinished).disabled(isUploading)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(isUploading ? "Uploading…" : "Upload") {
                        Task {
                            await model.upload(files)
                            if case .finished = model.state {
                                onFinished()
                            }
                        }
                    }
                    .disabled(isUploading || model.destination?.canUpload == false)
                }
            }
        }
        .interactiveDismissDisabled(isUploading)
        .frame(minWidth: 320, minHeight: 240)
    }

    private var isUploading: Bool {
        if case .uploading = model.state {
            return true
        }
        return false
    }
}
