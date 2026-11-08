import FolioKit
import PDFKit
import SwiftUI

/// A PDF view that says when it has laid out, so a requested page can wait for real bounds.
final class ReadingPDFView: PDFView {
    var onLayout: (() -> Void)?

    #if os(iOS)
        override func layoutSubviews() {
            super.layoutSubviews()
            onLayout?()
        }
    #else
        override func layout() {
            super.layout()
            onLayout?()
        }
    #endif
}

/// Keeps a PDF view's reading position: opens a document at a requested page, clamped to the loaded
/// file, and reports the 1-based page whenever it changes. A requested page waits until the view has
/// non-zero bounds, since PDFKit cannot scroll to a page before it has laid out.
@MainActor
final class PDFReadingPosition: NSObject {
    let view: ReadingPDFView
    var onPageChange: (Int) -> Void
    private(set) var url: URL?
    private var pendingPage: Int?

    init(view: ReadingPDFView = ReadingPDFView(), onPageChange: @escaping (Int) -> Void) {
        self.view = view
        self.onPageChange = onPageChange
        super.init()
        view.autoScales = true
        view.onLayout = { [weak self] in self?.applyPendingPage() }
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
        guard document != nil else { return }
        show(page: initialPage)
    }

    /// Moves to `page` (clamped), now if the view has laid out, otherwise at its first layout.
    func show(page: Int?) {
        pendingPage = page ?? 1
        applyPendingPage()
    }

    private func applyPendingPage() {
        guard let requested = pendingPage, let document = view.document, !view.bounds.isEmpty else { return }
        pendingPage = nil
        view.layoutDocumentView()
        let page = ReadingPosition.clamp(page: requested, pageCount: document.pageCount)
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

    final class Coordinator {
        let position: PDFReadingPosition
        var requestedPage: Int?

        init(position: PDFReadingPosition) {
            self.position = position
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(position: PDFReadingPosition(onPageChange: onPageChange))
    }

    private func update(_ coordinator: Coordinator) {
        let position = coordinator.position
        position.onPageChange = onPageChange
        if position.url != url {
            coordinator.requestedPage = initialPage
            position.load(PDFDocument(url: url), from: url, initialPage: initialPage)
        } else if let initialPage, initialPage != coordinator.requestedPage {
            // A continuation to the document already open still moves to its page.
            coordinator.requestedPage = initialPage
            position.show(page: initialPage)
        }
    }
}

#if os(iOS)
    extension PDFPreview: UIViewRepresentable {
        func makeUIView(context: Context) -> PDFView {
            update(context.coordinator)
            return context.coordinator.position.view
        }

        func updateUIView(_: PDFView, context: Context) {
            update(context.coordinator)
        }
    }
#else
    extension PDFPreview: NSViewRepresentable {
        func makeNSView(context: Context) -> PDFView {
            update(context.coordinator)
            return context.coordinator.position.view
        }

        func updateNSView(_: PDFView, context: Context) {
            update(context.coordinator)
        }
    }
#endif
