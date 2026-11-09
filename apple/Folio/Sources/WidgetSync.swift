import FolioKit
import Foundation
import WidgetKit

/// Keeps the snapshot the widgets read in step with the session and App lock. Does nothing unless the widget
/// extension is built in (`FOLIO_WIDGETS=YES`), so no document data reaches the App Group without a reader.
@MainActor
final class WidgetSync {
    static let isEnabled = Bundle.main.object(forInfoDictionaryKey: "FolioWidgetsEnabled") as? Bool == true

    /// Each refresh reads up to 300 documents, so app switching alone refreshes at most this often.
    static let minimumInterval: TimeInterval = 60

    private let store = WidgetSnapshotStore.shared()
    private var refreshTask: Task<Void, Never>?
    private var lastRefresh: Date?

    /// Pass `force` when the session or App lock changed, so the widgets follow at once.
    func update(session: SessionRestoration, revealsTitles: Bool, force: Bool = false) {
        guard Self.isEnabled else { return }
        let now = Date()
        if !force, let lastRefresh, now.timeIntervalSince(lastRefresh) < Self.minimumInterval { return }
        lastRefresh = now
        refreshTask?.cancel()
        switch session {
        case let .signedIn(url):
            let service = SharedConfiguration.makeService(for: url)
            refreshTask = Task { [store] in
                guard let snapshot = try? await WidgetSnapshotBuilder.make(from: service, revealsTitles: revealsTitles),
                    !Task.isCancelled
                else { return }
                store.save(snapshot)
                WidgetCenter.shared.reloadAllTimelines()
            }
        case .signedOut, .expired:
            store.save(.signedOut(at: Date()))
            WidgetCenter.shared.reloadAllTimelines()
        case .restoring, .interrupted, .unavailable:
            break
        }
    }

    /// Turning App lock on must not wait for the next refresh to take titles off the Home Screen.
    func hideTitles() {
        guard Self.isEnabled else { return }
        refreshTask?.cancel()
        store.hideTitles()
        WidgetCenter.shared.reloadAllTimelines()
    }
}
