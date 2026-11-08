import FolioKit
import PDFKit
import SwiftUI

/// One request to show a page. Each continuation makes a new one, so asking again for the page the
/// document was opened at still moves there after the reader scrolled away.
struct PDFPageRequest: Equatable {
    let id = UUID()
    let page: Int?
}

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
    private var lastRequest: PDFPageRequest?

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

    /// Shows the file at `url`, loading it with `document` only when it is not the one already shown, and
    /// honours `request` once: a request already applied does not move the view again.
    func open(_ url: URL, request: PDFPageRequest?, document: () -> PDFDocument?) {
        if self.url != url {
            lastRequest = request
            load(document(), from: url, initialPage: request?.page)
        } else if let request, request != lastRequest {
            lastRequest = request
            show(page: request.page)
        }
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
    let pageRequest: PDFPageRequest?
    let onPageChange: (Int) -> Void

    func makeCoordinator() -> PDFReadingPosition {
        PDFReadingPosition(onPageChange: onPageChange)
    }

    private func update(_ position: PDFReadingPosition) {
        position.onPageChange = onPageChange
        position.open(url, request: pageRequest) { PDFDocument(url: url) }
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
