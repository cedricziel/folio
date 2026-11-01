import QuickLook
import SwiftUI

#if os(iOS)
    struct QuickLookPreview: UIViewControllerRepresentable {
        let url: URL

        func makeCoordinator() -> Coordinator {
            Coordinator(url: url)
        }

        func makeUIViewController(context: Context) -> QLPreviewController {
            let controller = QLPreviewController()
            controller.dataSource = context.coordinator
            return controller
        }

        func updateUIViewController(_ controller: QLPreviewController, context: Context) {
            context.coordinator.url = url
            controller.reloadData()
        }

        final class Coordinator: NSObject, QLPreviewControllerDataSource {
            var url: URL
            init(url: URL) {
                self.url = url
            }

            func numberOfPreviewItems(in _: QLPreviewController) -> Int {
                1
            }

            func previewController(_: QLPreviewController, previewItemAt _: Int) -> any QLPreviewItem {
                url as NSURL
            }
        }
    }
#else
    import Quartz

    struct QuickLookPreview: NSViewRepresentable {
        let url: URL

        func makeNSView(context _: Context) -> QLPreviewView {
            let view = QLPreviewView(frame: .zero, style: .normal)!
            view.previewItem = url as NSURL
            return view
        }

        func updateNSView(_ view: QLPreviewView, context _: Context) {
            view.previewItem = url as NSURL
        }
    }
#endif
