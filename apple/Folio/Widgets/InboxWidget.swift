import FolioKit
import SwiftUI
import WidgetKit

struct InboxWidget: Widget {
    static let kind = "com.cedricziel.folio.inbox"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: Self.kind, provider: SnapshotProvider()) { entry in
            InboxWidgetView(entry: entry)
                .containerBackground(Color.paper, for: .widget)
                .widgetURL(WidgetLink.inbox.url)
        }
        .configurationDisplayName("Inbox")
        .description("How many documents are waiting for your review.")
        .supportedFamilies(Self.families)
    }

    private static var families: [WidgetFamily] {
        #if os(iOS)
            [.systemSmall, .accessoryCircular, .accessoryRectangular]
        #else
            [.systemSmall]
        #endif
    }
}

struct InboxWidgetView: View {
    let entry: FolioWidgetEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        switch entry.content {
        case let .documents(snapshot, isStale):
            switch family {
            #if os(iOS)
                case .accessoryCircular:
                    ZStack {
                        AccessoryWidgetBackground()
                        VStack(spacing: 0) {
                            Image(systemName: "tray").font(.caption)
                            Text(snapshot.inboxCountText).font(.headline).minimumScaleFactor(0.5)
                        }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("\(snapshot.inboxCountText) documents to review")
                case .accessoryRectangular:
                    VStack(alignment: .leading) {
                        Label("Inbox", systemImage: "tray").font(.headline).widgetAccentable()
                        Text("\(snapshot.inboxCountText) to review")
                    }
            #endif
            default:
                VStack(alignment: .leading, spacing: 2) {
                    Label("Inbox", systemImage: "tray")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(Color.archive)
                        .widgetAccentable()
                    Spacer(minLength: 0)
                    Text(snapshot.inboxCountText)
                        .font(.system(size: 44, weight: .semibold, design: .rounded))
                        .minimumScaleFactor(0.5)
                        .contentTransition(.numericText())
                        .widgetAccentable()
                    Text(snapshot.inboxCount == 1 ? "document to review" : "documents to review")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if isStale {
                        Text("Updated \(snapshot.updatedAt, style: .relative) ago")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            }
        case .notSetUp, .signedOut:
            WidgetMessage(content: entry.content)
        }
    }
}
