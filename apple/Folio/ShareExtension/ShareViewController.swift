import FolioKit
import SwiftUI
import UniformTypeIdentifiers

/// Shared by the iOS and macOS share extensions: turns the shared attachments into files to upload.
enum ShareItems {
    static let accepted: [UTType] = [.pdf, .image, .data]

    @MainActor static func files(from context: NSExtensionContext?) async -> [UploadFile] {
        let providers = (context?.inputItems as? [NSExtensionItem])?.flatMap { $0.attachments ?? [] } ?? []
        var files: [UploadFile] = []
        for provider in providers {
            if let file = try? await load(provider) {
                files.append(file)
            }
        }
        return files
    }

    @MainActor private static func load(_ provider: NSItemProvider) async throws -> UploadFile? {
        guard
            let type = accepted.first(where: { provider.hasItemConformingToTypeIdentifier($0.identifier) })
        else { return nil }
        let url = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<URL, any Error>) in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                guard let url else { return continuation.resume(throwing: error ?? FolioError.unreadableResponse) }
                // The provided file is deleted when this handler returns, so copy it first.
                let copy = FileManager.default.temporaryDirectory
                    .appending(path: "\(UUID().uuidString)-\(url.lastPathComponent)")
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
        if let suggested = provider.suggestedName, !url.pathExtension.isEmpty,
            !suggested.hasSuffix(".\(url.pathExtension)")
        {
            file.filename = "\(suggested).\(url.pathExtension)"
        }
        return file
    }
}

struct ShareRoot: View {
    let context: NSExtensionContext?
    @State private var model: ShareViewModel?
    @State private var loaded = false

    var body: some View {
        Group {
            if let model {
                ShareView(model: model) { context?.completeRequest(returningItems: nil) }
            } else if loaded {
                VStack(spacing: FolioTheme.Spacing.space3) {
                    Text("Open Folio and enter your server address first.")
                        .font(FolioTheme.Typography.body)
                    Button("Close") { context?.completeRequest(returningItems: nil) }
                }
                .padding(FolioTheme.Spacing.space6)
            } else {
                Text("Reading the shared files…")
                    .font(FolioTheme.Typography.body)
                    .foregroundStyle(Color.inkMuted)
                    .padding(FolioTheme.Spacing.space6)
            }
        }
        .task {
            let files = await ShareItems.files(from: context)
            model = files.isEmpty ? nil : await buildModel(files)
            loaded = true
        }
    }

    private func buildModel(_ files: [UploadFile]) async -> ShareViewModel? {
        guard let serverURL = SharedConfiguration.serverURL(), let service = SharedConfiguration.makeService() else {
            return nil
        }
        let uploader = BackgroundUploader(
            serverURL: serverURL, token: await SharedConfiguration.tokenProvider().token(),
            identifier: "com.cedricziel.folio.share.\(UUID().uuidString)"
        )
        return await ShareViewModel(files: files, service: service, background: uploader)
    }
}

#if os(iOS)
    import UIKit

    final class ShareViewController: UIViewController {
        override func viewDidLoad() {
            super.viewDidLoad()
            let host = UIHostingController(rootView: ShareRoot(context: extensionContext))
            addChild(host)
            host.view.frame = view.bounds
            host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            view.addSubview(host.view)
            host.didMove(toParent: self)
        }
    }
#else
    import AppKit

    final class ShareViewController: NSViewController {
        override func loadView() {
            let host = NSHostingView(rootView: ShareRoot(context: extensionContext))
            host.frame = NSRect(x: 0, y: 0, width: 360, height: 420)
            view = host
        }
    }
#endif
