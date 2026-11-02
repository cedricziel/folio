import CoreGraphics
import FolioKit
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

typealias Document = FolioKit.Document

@main struct ThumbnailFixtureApp: App {
    var body: some Scene {
        WindowGroup { ThumbnailFixture() }
    }
}

@MainActor private struct ThumbnailFixture: View {
    private static let documents = [
        Document(title: "PDF preview", originalFilename: "invoice.pdf", mimeType: "application/pdf"),
        Document(title: "Image preview", originalFilename: "photo.jpg", mimeType: "image/jpeg"),
        Document(title: "Not processed yet", originalFilename: "pending.pdf", mimeType: "application/pdf"),
        Document(title: "Authorization unavailable", originalFilename: "private.pdf", mimeType: "application/pdf"),
    ]
    private let service: MockFolioService
    private let cache: ThumbnailCache
    @State private var model: DocumentListViewModel
    @State private var ready = false
    @State private var opened: String?

    init() {
        let service = MockFolioService(documents: Self.documents)
        self.service = service
        cache = ThumbnailCache(service: service, vaultID: MockFolioService.defaultVault.id)
        _model = State(initialValue: DocumentListViewModel(service: service))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space4) {
            Text("Document thumbnails").font(FolioTheme.Typography.heading)
            ForEach(model.rows) { row in
                Button {
                    opened = row.title
                } label: {
                    DocumentRowView(row: row)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier(row.title)
            }
            if ready { Text("Previews ready") }
            if let opened { Text("Opened: \(opened)") }
        }
        .padding(FolioTheme.Spacing.space4)
        .frame(maxWidth: 560)
        .environment(\.thumbnailCache, ProcessInfo.processInfo.arguments.contains("--without-thumbnails") ? nil : cache)
        .task {
            let jpeg = Self.preview()
            await service.setThumbnail(jpeg, for: Self.documents[0].id)
            await service.setThumbnail(jpeg, for: Self.documents[1].id)
            await service.setThumbnail(nil, for: Self.documents[3].id, error: .signInRequired)
            await model.load()
            _ = try? await cache.data(for: Self.documents[0])
            _ = try? await cache.data(for: Self.documents[1])
            ready = true
        }
    }

    private static func preview() -> Data {
        let context = CGContext(
            data: nil, width: 120, height: 170, bitsPerComponent: 8, bytesPerRow: 480,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
        )!
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 120, height: 170))
        context.setFillColor(CGColor(red: 0.12, green: 0.35, blue: 0.65, alpha: 1))
        context.fill(CGRect(x: 12, y: 130, width: 96, height: 26))
        context.setFillColor(CGColor(gray: 0.4, alpha: 1))
        for y in stride(from: 35, through: 110, by: 15) {
            context.fill(CGRect(x: 12, y: CGFloat(y), width: 80, height: 4))
        }
        let data = NSMutableData()
        let destination = CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        precondition(CGImageDestinationFinalize(destination))
        return data as Data
    }
}
