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
    private var loadedTags: [Tag] = []
    public private(set) var state = State.idle
    public let files: [UploadFile]
    public let destination: VaultDestinationViewModel

    private let background: any BackgroundUploading
    private var tagsVaultID: UUID?

    public var tags: [Tag] {
        tagsVaultID == destination.selectedVaultID ? loadedTags : []
    }

    public init(
        files: [UploadFile], service: any FolioService, background: any BackgroundUploading,
        destination: VaultDestinationViewModel? = nil
    ) {
        self.files = files
        self.background = background
        self.destination = destination ?? VaultDestinationViewModel(service: service)
        title = files.count == 1 ? (files[0].filename as NSString).deletingPathExtension : ""
    }

    public func loadTags() async {
        await destination.load()
        let vaultID = destination.selectedVaultID
        loadedTags = []
        selectedTagIDs = []
        guard destination.canTag else { return }
        let loaded = (try? await destination.service.tags()) ?? []
        guard vaultID == destination.selectedVaultID, !Task.isCancelled else { return }
        loadedTags = loaded
        tagsVaultID = vaultID
    }

    /// Tags can only be attached once the server has created the document, so tagged uploads run in the
    /// foreground. Untagged ones are handed to the background session and survive the sheet closing.
    public func upload() async {
        guard state != .uploading else { return }
        state = .uploading
        await destination.load()
        guard destination.canUpload else {
            state = .failed(destination.error ?? "Choose a vault you can upload to.")
            return
        }
        let service = destination.service
        let vaultID = destination.selectedVaultID
        let selectedTagIDs = destination.canTag && tagsVaultID == vaultID ? selectedTagIDs : []
        let files = titled()
        do {
            if selectedTagIDs.isEmpty {
                for file in files {
                    try background.enqueue(file, vaultID: vaultID)
                }
            } else {
                for file in files {
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
