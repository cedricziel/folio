import FolioKit
import SwiftUI
import WidgetKit

struct LatestDocumentWidget: Widget {
    static let kind = "com.cedricziel.folio.latest-document"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: Self.kind, provider: SnapshotProvider()) { entry in
            LatestDocumentWidgetView(entry: entry)
                .containerBackground(Color.paper, for: .widget)
        }
        .configurationDisplayName("Latest document")
        .description("The newest document in Folio, with a button to scan another.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

struct LatestDocumentWidgetView: View {
    let entry: FolioWidgetEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        switch entry.content {
        case let .documents(snapshot, _):
            HStack(alignment: .top) {
                latest(snapshot)
                if family == .systemMedium {
                    Spacer()
                    Button(intent: ScanDocumentIntent()) {
                        Label("Scan", systemImage: "doc.viewfinder")
                    }
                    .tint(Color.archive)
                }
            }
            .widgetURL(snapshot.latest.map { WidgetLink.document($0.documentID, vaultID: $0.vaultID).url })
        case .notSetUp, .signedOut:
            WidgetMessage(content: entry.content)
        }
    }

    private func latest(_ snapshot: WidgetSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("Latest", systemImage: "doc.text")
                .font(.caption.weight(.semibold))
                .foregroundStyle(Color.archive)
                .widgetAccentable()
            Spacer(minLength: 0)
            if let latest = snapshot.latest {
                // App lock keeps the title out of the snapshot; the date alone says something arrived.
                Text(latest.title ?? "Locked document")
                    .font(.headline)
                    .lineLimit(3)
                    .privacySensitive()
                Text(latest.createdAt, style: .date)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("No documents yet")
                    .font(.headline)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }
}
