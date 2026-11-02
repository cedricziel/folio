import FolioKit
import SwiftUI

private struct ThumbnailCacheKey: EnvironmentKey {
    static let defaultValue: ThumbnailCache? = nil
}

extension EnvironmentValues {
    var thumbnailCache: ThumbnailCache? {
        get { self[ThumbnailCacheKey.self] }
        set { self[ThumbnailCacheKey.self] = newValue }
    }
}

struct DocumentRowView: View {
    @Environment(\.thumbnailCache) private var thumbnails
    @State private var preview: Image?

    private struct ThumbnailIdentity: Hashable {
        let cache: ObjectIdentifier?
        let id: UUID
        let revision: Date
    }

    let row: DocumentRow

    var body: some View {
        HStack(alignment: .top, spacing: FolioTheme.Spacing.space3) {
            PaperThumbnail(showSymbol: preview == nil)
                .overlay {
                    if let preview {
                        preview.resizable().scaledToFit()
                    }
                }
                .frame(width: 44, height: 44 * 1.414)
                .clipShape(RoundedRectangle(cornerRadius: FolioTheme.Radius.radiusSm))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                Text(row.title)
                    .font(FolioTheme.Typography.heading)
                    .foregroundStyle(Color.ink)
                    .lineLimit(2)
                if let subtitle = row.subtitle {
                    Text(subtitle)
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.inkMuted)
                        .lineLimit(1)
                }
                if let status = row.statusText {
                    Text(status)
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.pending)
                        .lineLimit(1)
                } else if !row.tags.isEmpty {
                    HStack(spacing: FolioTheme.Spacing.space2) {
                        ForEach(row.tags) { TagCapsule(tag: $0) }
                    }
                }
            }
            Spacer(minLength: FolioTheme.Spacing.space2)
            Text(row.dateText)
                .font(FolioTheme.Typography.meta)
                .foregroundStyle(Color.inkMuted)
                .lineLimit(1)
                .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.vertical, FolioTheme.Spacing.space1)
        .accessibilityElement(children: .combine)
        .task(
            id: ThumbnailIdentity(
                cache: thumbnails.map(ObjectIdentifier.init), id: row.id, revision: row.document.updatedAt)
        ) {
            preview = nil
            guard let thumbnails else { return }
            do {
                guard let data = try await thumbnails.data(for: row.document) else { return }
                try Task.checkCancellation()
                #if os(iOS)
                    if let image = UIImage(data: data) { preview = Image(uiImage: image) }
                #else
                    if let image = NSImage(data: data) { preview = Image(nsImage: image) }
                #endif
            } catch {
                // Thumbnail failure must not block opening the original document.
            }
        }
    }
}

/// The placeholder used while a preview is loading or unavailable.
struct PaperThumbnail: View {
    var showSymbol = true

    var body: some View {
        RoundedRectangle(cornerRadius: FolioTheme.Radius.radiusSm)
            .fill(Color.paperRaised)
            .aspectRatio(1 / 1.414, contentMode: .fit)
            .overlay {
                if showSymbol {
                    Image(systemName: "doc.text")
                        .foregroundStyle(Color.inkMuted)
                }
            }
            .overlay(
                RoundedRectangle(cornerRadius: FolioTheme.Radius.radiusSm)
                    .stroke(Color.rule, lineWidth: 1)
            )
            .shadow(color: .black.opacity(0.08), radius: 4, y: 2)
            .accessibilityHidden(true)
    }
}

struct TagCapsule: View {
    let tag: Tag

    var body: some View {
        Label(tag.name, systemImage: "tag")
            .labelStyle(.titleOnly)
            .font(FolioTheme.Typography.bodySm)
            .foregroundStyle(Color.ink)
            .padding(.horizontal, FolioTheme.Spacing.space2)
            .padding(.vertical, FolioTheme.Spacing.space1 / 2)
            .background(tint, in: .capsule)
    }

    private var tint: Color {
        switch TagTint.tint(forTagNamed: tag.name) {
        case .sage: .tagSage
        case .slate: .tagSlate
        case .clay: .tagClay
        case .ochre: .tagOchre
        case .plum: .tagPlum
        case .sky: .tagSky
        }
    }
}
