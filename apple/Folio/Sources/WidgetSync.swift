import FolioKit
import Foundation
import WidgetKit

#if os(iOS)
    import UIKit
#endif

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
            #if os(iOS)
                // The refresh on going to the background picks up what changed in the app; without this, iOS can
                // suspend the app before the snapshot is saved.
                let assertion = BackgroundAssertion(name: "Folio widget refresh") { [weak self] in
                    self?.refreshTask?.cancel()
                }
            #endif
            refreshTask = Task { [store] in
                #if os(iOS)
                    defer { assertion.end() }
                #endif
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

#if os(iOS)
    /// Keeps the app running in the background until `end()`, or until the system's time runs out.
    @MainActor
    private final class BackgroundAssertion {
        private var identifier = UIBackgroundTaskIdentifier.invalid

        init(name: String, onExpiry: @escaping @MainActor @Sendable () -> Void) {
            identifier = UIApplication.shared.beginBackgroundTask(withName: name) { [weak self] in
                onExpiry()
                self?.end()
            }
        }

        func end() {
            guard identifier != .invalid else { return }
            UIApplication.shared.endBackgroundTask(identifier)
            identifier = .invalid
        }
    }
#endif
