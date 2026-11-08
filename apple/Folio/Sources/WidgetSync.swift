import FolioKit
import Foundation
import WidgetKit

/// Keeps the snapshot the widgets read in step with the session and App lock. Does nothing unless the widget
/// extension is built in (`FOLIO_WIDGETS=YES`), so no document data reaches the App Group without a reader.
@MainActor
final class WidgetSync {
    static let isEnabled = Bundle.main.object(forInfoDictionaryKey: "FolioWidgetsEnabled") as? Bool == true

    private let store = WidgetSnapshotStore.shared()
    private var refreshTask: Task<Void, Never>?

    func update(session: SessionRestoration, revealsTitles: Bool) {
        guard Self.isEnabled else { return }
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
