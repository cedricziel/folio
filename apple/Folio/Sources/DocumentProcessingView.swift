import FolioKit
import SwiftUI

struct DocumentProcessingView: View {
    let document: Document

    var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
            Text("Processing")
                .accessibilityAddTraits(.isHeader)
                .font(FolioTheme.Typography.heading)
                .foregroundStyle(Color.ink)
            ForEach(document.processingSummary) { stage in
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                    Text(stage.title)
                        .font(FolioTheme.Typography.label)
                        .foregroundStyle(Color.inkMuted)
                    Text(stage.status)
                        .font(FolioTheme.Typography.body)
                        .foregroundStyle(Color.ink)
                    if let detail = stage.detail {
                        Text(detail)
                            .font(FolioTheme.Typography.body)
                            .foregroundStyle(Color.inkMuted)
                    }
                }
                .accessibilityElement(children: .combine)
            }
        }
        .telemetryScreen(.documentProcessing)
    }
}
