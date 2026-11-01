import Foundation
import Observation

/// Drives the compact share sheet: title, tag picker and "Upload".
@MainActor @Observable
public final class ShareViewModel {
    public enum State: Equatable {
        case idle
        case uploading
        case done
        case failed(String)
    }

    public var title: String
    public var selectedTagIDs: Set<Tag.ID> = []
    public private(set) var tags: [Tag] = []
    public private(set) var state = State.idle
    public let files: [UploadFile]

    private let service: any FolioService
    private let background: any BackgroundUploading

    public init(files: [UploadFile], service: any FolioService, background: any BackgroundUploading) {
        self.files = files
        self.service = service
        self.background = background
        title = files.count == 1 ? (files[0].filename as NSString).deletingPathExtension : ""
    }

    public func loadTags() async {
        tags = (try? await service.tags()) ?? []
    }

    /// Tags can only be attached once the server has created the document, so tagged uploads run in the
    /// foreground. Untagged ones are handed to the background session and survive the sheet closing.
    public func upload() async {
        state = .uploading
        do {
            if selectedTagIDs.isEmpty {
                for file in titled() {
                    try background.enqueue(file)
                }
            } else {
                for file in titled() {
                    let document = try await service.upload(file)
                    for tagID in selectedTagIDs {
                        try await service.addTag(tagID, to: document.id)
                    }
                }
            }
            state = .done
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private func titled() -> [UploadFile] {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard files.count == 1, !trimmed.isEmpty else { return files }
        var file = files[0]
        file.title = trimmed
        return [file]
    }
}
