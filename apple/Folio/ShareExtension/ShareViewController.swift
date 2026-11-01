import FolioKit
import UIKit
import UniformTypeIdentifiers

/// Uploads PDFs and images shared from other apps to the server chosen in Folio.
/// Minimal on purpose: no UI beyond a status line, and it relies on the App Group to read the server URL.
final class ShareViewController: UIViewController {
    private let statusLabel = UILabel()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        statusLabel.numberOfLines = 0
        statusLabel.textAlignment = .center
        statusLabel.font = .preferredFont(forTextStyle: .body)
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(statusLabel)
        NSLayoutConstraint.activate([
            statusLabel.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            statusLabel.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 24),
            statusLabel.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -24),
        ])
        Task { await shareAll() }
    }

    private func shareAll() async {
        guard let serverURL = UserDefaultsServerConfigStore.shared().load() else {
            return finish("Open Folio and enter your server address first.")
        }
        let providers =
            (extensionContext?.inputItems as? [NSExtensionItem])?
            .flatMap { $0.attachments ?? [] } ?? []
        statusLabel.text = "Filing…"

        let service = LiveFolioService(baseURL: serverURL)
        var filed = 0
        do {
            for provider in providers {
                guard let file = try await loadFile(from: provider) else { continue }
                _ = try await service.upload(file)
                filed += 1
            }
            finish(filed == 0 ? "Nothing to file." : "Filed \(filed) \(filed == 1 ? "document" : "documents").")
        } catch {
            finish(error.localizedDescription)
        }
    }

    private func loadFile(from provider: NSItemProvider) async throws -> UploadFile? {
        guard
            let type = [UTType.pdf, .image].first(where: { provider.hasItemConformingToTypeIdentifier($0.identifier) })
        else { return nil }
        let url = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<URL, any Error>) in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                guard let url else { return continuation.resume(throwing: error ?? FolioError.unreadableResponse) }
                // The provided file is deleted when this handler returns, so copy it first.
                let copy = FileManager.default.temporaryDirectory.appending(
                    path: "\(UUID().uuidString)-\(url.lastPathComponent)")
                do {
                    try FileManager.default.copyItem(at: url, to: copy)
                    continuation.resume(returning: copy)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
        defer { try? FileManager.default.removeItem(at: url) }
        var file = try UploadFile(contentsOf: url)
        file.filename = provider.suggestedName.map { "\($0).\(url.pathExtension)" } ?? file.filename
        return file
    }

    private func finish(_ message: String) {
        statusLabel.text = message
        Task {
            try? await Task.sleep(for: .seconds(1.5))
            extensionContext?.completeRequest(returningItems: nil)
        }
    }
}
