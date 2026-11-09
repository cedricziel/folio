import FolioKit
import SwiftUI

#if os(iOS)
    import SafariServices
#endif

/// A continuation the app explains instead of opening, with the buttons it offers.
struct HandoffAlert: Identifiable {
    let id = UUID()
    let title: String
    let message: String
    let actions: [HandoffPresentation.Action]
}

#if os(iOS)
    struct BrowserPage: Identifiable {
        let url: URL
        var id: URL { url }
    }

    /// Safari inside the app. It never passes a universal link back to Folio, so a link cannot loop.
    struct SafariView: UIViewControllerRepresentable {
        let url: URL

        func makeUIViewController(context: Context) -> SFSafariViewController {
            SFSafariViewController(url: url)
        }

        func updateUIViewController(_: SFSafariViewController, context: Context) {}
    }
#endif
