import FolioKit
import PDFKit
import SwiftUI

/// Keeps a PDF view's reading position: opens a document once at a requested page, clamped to the
/// loaded file, and reports the 1-based page whenever it changes.
@MainActor
final class PDFReadingPosition: NSObject {
    let view: PDFView
    var onPageChange: (Int) -> Void
    private(set) var url: URL?

    init(view: PDFView = PDFView(), onPageChange: @escaping (Int) -> Void) {
        self.view = view
        self.onPageChange = onPageChange
        super.init()
        view.autoScales = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(pageChanged), name: .PDFViewPageChanged, object: view)
    }

    var currentPage: Int? {
        guard let page = view.currentPage, let document = view.document else { return nil }
        let index = document.index(for: page)
        return index == NSNotFound ? nil : index + 1
    }

    func load(_ document: PDFDocument?, from url: URL? = nil, initialPage: Int?) {
        self.url = url
        view.document = document
        guard let document else { return }
        view.layoutDocumentView()
        let page = ReadingPosition.clamp(page: initialPage, pageCount: document.pageCount)
        if let target = document.page(at: page - 1) { view.go(to: target) }
        publish()
    }

    @objc private func pageChanged(_: Notification) {
        publish()
    }

    /// Reports after the current view update, since loading happens while SwiftUI builds the view.
    private func publish() {
        Task { @MainActor [weak self] in
            guard let self, let page = self.currentPage else { return }
            self.onPageChange(page)
        }
    }
}

/// A PDF shown with PDFKit, so Handoff can carry and restore its page. Other files use QuickLook.
struct PDFPreview {
    let url: URL
    let initialPage: Int?
    let onPageChange: (Int) -> Void

    func makeCoordinator() -> PDFReadingPosition {
        PDFReadingPosition(onPageChange: onPageChange)
    }

    private func update(_ position: PDFReadingPosition) {
        position.onPageChange = onPageChange
        guard position.url != url else { return }
        position.load(PDFDocument(url: url), from: url, initialPage: position.url == nil ? initialPage : nil)
    }
}

#if os(iOS)
    extension PDFPreview: UIViewRepresentable {
        func makeUIView(context: Context) -> PDFView {
            update(context.coordinator)
            return context.coordinator.view
        }

        func updateUIView(_: PDFView, context: Context) {
            update(context.coordinator)
        }
    }
#else
    extension PDFPreview: NSViewRepresentable {
        func makeNSView(context: Context) -> PDFView {
            update(context.coordinator)
            return context.coordinator.view
        }

        func updateNSView(_: PDFView, context: Context) {
            update(context.coordinator)
        }
    }
#endif
