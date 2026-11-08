import FolioKit
import SwiftUI

struct ShareRoot: View {
    let context: NSExtensionContext?
    @State private var model: ShareViewModel?
    @State private var loaded = false
    @State private var attachments: SharedAttachmentLoader.Result?
    @State private var attempt = 0

    var body: some View {
        Group {
            if let model {
                ShareView(model: model) { finish(model.state == .done ? nil : .shareCancel) }
            } else if loaded, let attachments, !attachments.canUpload {
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space3) {
                    Text(attachments.files.isEmpty ? "No readable attachments" : "Some attachments could not be read")
                        .font(FolioTheme.Typography.heading)
                    Text("Nothing has been uploaded. Retry or cancel and share the files again.")
                    ScrollView {
                        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                            ForEach(Array(attachments.failures.enumerated()), id: \.offset) { _, failure in
                                Text("\(failure.name): \(failure.message)")
                            }
                        }
                    }
                    HStack {
                        Button("Cancel", role: .cancel) { finish(.shareCancel) }
                        Button("Retry") { attempt += 1 }
                    }
                }
                .padding(FolioTheme.Spacing.space6)
            } else if loaded {
                VStack(spacing: FolioTheme.Spacing.space3) {
                    Text("Open Folio and enter your server address first.")
                        .font(FolioTheme.Typography.body)
                    Button("Close") { finish() }
                }
                .padding(FolioTheme.Spacing.space6)
            } else {
                Text("Reading the shared files…")
                    .font(FolioTheme.Typography.body)
                    .foregroundStyle(Color.inkMuted)
                    .padding(FolioTheme.Spacing.space6)
            }
        }
        .telemetryScreen(.share)
        .task(id: attempt) {
            loaded = false
            let providers = (context?.inputItems as? [NSExtensionItem])?.flatMap { $0.attachments ?? [] } ?? []
            let result = await SharedAttachmentLoader.load(providers)
            attachments = result
            model = result.canUpload ? await buildModel(result.files) : nil
            loaded = true
        }
    }

    /// Sends recorded telemetry, waiting briefly, before the system suspends the extension.
    private func finish(_ action: TelemetryAction? = nil) {
        if let action { FolioTelemetry.shared.action(action) }
        Task {
            await FolioTelemetry.shared.flush(timeout: .milliseconds(1500))
            context?.completeRequest(returningItems: nil)
        }
    }

    private func buildModel(_ files: [UploadFile]) async -> ShareViewModel? {
        guard let serverURL = SharedConfiguration.serverURL(), let service = SharedConfiguration.makeService() else {
            return nil
        }
        guard let token = try? await SharedConfiguration.tokenProvider(for: serverURL).token() else { return nil }
        let uploader = BackgroundUploader(
            serverURL: serverURL, token: token,
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
