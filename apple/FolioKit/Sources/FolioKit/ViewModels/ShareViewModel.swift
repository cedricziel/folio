import Foundation
import Observation

/// Drives the compact share sheet: title, tag picker and "Upload".
@MainActor @Observable
public final class ShareViewModel {
    public enum State: Equatable {
        case idle
        case uploading
        /// Handed to the background session; the outcome arrives in `retained` later.
        case queued
        /// The server accepted every file.
        case done
        case failed(String)
    }

    public var title: String
    public var selectedTagIDs: Set<Tag.ID> = []
    private var loadedTags: [Tag] = []
    public private(set) var state = State.idle
    /// Earlier background shares the server has not accepted, plus the outcome of retries started here.
    public private(set) var retained: [ShareUploadRecord] = []
    public private(set) var retrying: Set<ShareUploadRecord.ID> = []
    public private(set) var retryError: String?
    private var retried: Set<ShareUploadRecord.ID> = []
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
        title = files.count == 1 ? files[0].filename : ""
        refreshRetained()
    }

    public func refreshRetained() {
        retained = background.retainedUploads().filter { $0.status != .uploaded || retried.contains($0.id) }
    }

    public func retry(_ id: ShareUploadRecord.ID) async {
        guard !retrying.contains(id) else { return }
        retrying.insert(id)
        retryError = nil
        defer { retrying.remove(id) }
        do {
            try await background.retry(id)
            retried.insert(id)
        } catch {
            retryError = error.localizedDescription
        }
        refreshRetained()
    }

    public func discard(_ id: ShareUploadRecord.ID) {
        background.discard(id)
        retried.remove(id)
        refreshRetained()
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
                    try await background.enqueue(file, vaultID: vaultID)
                }
                state = .queued
            } else {
                for file in files {
                    let document = try await service.upload(file)
                    for tagID in selectedTagIDs {
                        try await service.addTag(tagID, to: document.id)
                    }
                }
                state = .done
            }
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
