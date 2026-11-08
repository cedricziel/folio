import AppIntents
import FolioKit
import SwiftUI
import WidgetKit

@main
struct FolioWidgets: WidgetBundle {
    var body: some Widget {
        InboxWidget()
        LatestDocumentWidget()
        if #available(iOS 18.0, macOS 26.0, *) {
            ScanControl()
        }
    }
}

/// Pulls FolioKit's intents into the extension's metadata, so the control and buttons can run `ScanDocumentIntent`.
struct FolioWidgetIntents: AppIntentsPackage {
    static var includedPackages: [any AppIntentsPackage.Type] { [FolioIntentsPackage.self] }
}

struct SnapshotProvider: TimelineProvider {
    func placeholder(in _: Context) -> FolioWidgetEntry {
        FolioWidgetEntry(date: Date(), content: .documents(.preview, isStale: false))
    }

    func getSnapshot(in context: Context, completion: @escaping (FolioWidgetEntry) -> Void) {
        if context.isPreview {
            completion(placeholder(in: context))
        } else {
            completion(FolioWidgetEntry.timeline(for: WidgetSnapshotStore.shared().load(), now: Date())[0])
        }
    }

    /// The app reloads timelines whenever it writes a new snapshot, so the widget never needs to ask.
    func getTimeline(in _: Context, completion: @escaping (Timeline<FolioWidgetEntry>) -> Void) {
        let entries = FolioWidgetEntry.timeline(for: WidgetSnapshotStore.shared().load(), now: Date())
        completion(Timeline(entries: entries, policy: .never))
    }
}

extension FolioWidgetEntry: @retroactive TimelineEntry {}

extension WidgetSnapshot {
    static let preview = WidgetSnapshot(
        isSignedIn: true, inboxCount: 4, inboxCountIsPartial: false,
        latest: .init(documentID: UUID(), vaultID: UUID(), title: "Electricity bill", createdAt: Date()),
        hidesTitles: false, updatedAt: Date())
}

/// What a widget shows when it has no documents to show.
struct WidgetMessage: View {
    let content: FolioWidgetEntry.Content

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Image(systemName: content == .signedOut ? "person.crop.circle.badge.exclamationmark" : "tray")
                .font(.title3)
                .foregroundStyle(Color.archive)
                .widgetAccentable()
            Text(content == .signedOut ? "Signed out" : "Open Folio")
                .font(.headline)
            Text(content == .signedOut ? "Sign in to Folio to see your inbox." : "Open Folio once to show your inbox.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
