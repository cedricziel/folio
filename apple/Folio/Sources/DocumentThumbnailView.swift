import FolioKit
import ImageIO
import SwiftUI

struct DocumentThumbnailView: View {
    let document: Document
    @Environment(\.thumbnailCache) private var thumbnails

    private struct Identity: Hashable {
        let cache: ObjectIdentifier?
        let documentID: UUID
        let revision: Date
    }
    @State private var image: CGImage?

    var body: some View {
        Group {
            if let image {
                Image(decorative: image, scale: 1)
                    .resizable()
                    .scaledToFit()
            } else {
                PaperThumbnail()
            }
        }
        .accessibilityHidden(true)
        .task(
            id: Identity(
                cache: thumbnails.map(ObjectIdentifier.init), documentID: document.id, revision: document.updatedAt)
        ) {
            image = nil
            guard let thumbnails,
                let data = try? await thumbnails.data(for: document),
                !Task.isCancelled,
                let source = CGImageSourceCreateWithData(data as CFData, nil),
                let preview = CGImageSourceCreateImageAtIndex(source, 0, nil)
            else { return }
            image = preview
        }
    }
}
