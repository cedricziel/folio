import FolioKit
import SwiftUI

struct DocumentRowView: View {
    let row: DocumentRow

    var body: some View {
        HStack(alignment: .top, spacing: FolioTheme.Spacing.space3) {
            PaperThumbnail()
                .frame(width: 44)
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
        }
        .padding(.vertical, FolioTheme.Spacing.space1)
        .accessibilityElement(children: .combine)
    }
}

/// Folio has no thumbnail endpoint yet, so every document shows an A4-shaped sheet.
struct PaperThumbnail: View {
    var body: some View {
        RoundedRectangle(cornerRadius: FolioTheme.Radius.radiusSm)
            .fill(Color.paperRaised)
            .aspectRatio(1 / 1.414, contentMode: .fit)
            .overlay {
                Image(systemName: "doc.text")
                    .foregroundStyle(Color.inkMuted)
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
