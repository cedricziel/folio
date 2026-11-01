#if os(iOS)
    import SwiftUI
    import VisionKit

    /// Wraps `VNDocumentCameraViewController`. Pages come back as images; `ScanReviewViewModel` turns them into a PDF.
    struct DocumentScanner: UIViewControllerRepresentable {
        static var isSupported: Bool {
            VNDocumentCameraViewController.isSupported
        }

        let onFinish: ([CGImage]) -> Void
        let onCancel: () -> Void

        func makeCoordinator() -> Coordinator {
            Coordinator(parent: self)
        }

        func makeUIViewController(context: Context) -> VNDocumentCameraViewController {
            let controller = VNDocumentCameraViewController()
            controller.delegate = context.coordinator
            return controller
        }

        func updateUIViewController(_: VNDocumentCameraViewController, context _: Context) {}

        final class Coordinator: NSObject, VNDocumentCameraViewControllerDelegate {
            let parent: DocumentScanner
            init(parent: DocumentScanner) {
                self.parent = parent
            }

            func documentCameraViewController(
                _: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan
            ) {
                let pages = (0..<scan.pageCount).compactMap { scan.imageOfPage(at: $0).cgImage }
                parent.onFinish(pages)
            }

            func documentCameraViewControllerDidCancel(_: VNDocumentCameraViewController) {
                parent.onCancel()
            }

            func documentCameraViewController(
                _: VNDocumentCameraViewController, didFailWithError _: any Error
            ) {
                parent.onCancel()
            }
        }
    }
#endif
