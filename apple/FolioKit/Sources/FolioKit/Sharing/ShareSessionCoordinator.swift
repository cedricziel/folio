import Foundation

/// Reconnects to share-extension upload sessions from whichever process is running: when the system relaunches
/// the app to deliver their events, and whenever the app or the share sheet opens, so a share whose upload task
/// is gone (the app was force-quit, the extension died before the task started) becomes retryable instead of
/// staying "Uploading in the background" forever.
@MainActor
public final class ShareSessionCoordinator {
    public static let shared = ShareSessionCoordinator(store: .shared()) { identifier in
        BackgroundUploader(serverURL: nil, tokens: nil, identifier: identifier)
    }

    /// A pending upload is only judged once it is this old, so a sheet that just started one is left alone.
    public static let grace: TimeInterval = 2 * 60

    private let store: ShareUploadStore
    private let attach: (String) -> BackgroundUploader
    private var uploaders: [String: BackgroundUploader] = [:]
    private var completions: [String: () -> Void] = [:]

    /// Called when a reattached session reports the outcome of an upload.
    public var onChange: (@MainActor (ShareUploadRecord) -> Void)?

    /// - Parameter attach: Creates an uploader for an existing session identifier, without a server to send to.
    public init(store: ShareUploadStore, attach: @escaping (String) -> BackgroundUploader) {
        self.store = store
        self.attach = attach
    }

    /// Takes over the events the system delivers for a share session after the extension has gone. Returns false
    /// for sessions that are not share uploads; the caller then completes them itself.
    public func handleEvents(for identifier: String, completion: @escaping () -> Void) -> Bool {
        guard identifier.hasPrefix(BackgroundUploader.identifierPrefix) else { return false }
        completions[identifier] = completion
        let uploader = uploader(for: identifier)
        uploader.onEventsFinished = {
            Task { @MainActor [weak self] in self?.finishEvents(for: identifier) }
        }
        return true
    }

    /// Fails stale pending uploads that never got a session and, with `reattachingSessions`, reattaches every
    /// session that still has pending uploads older than `grace` and fails those without a task.
    ///
    /// Only the app may reattach: a background session belongs to one process at a time, and an extension that
    /// opens one the app (or another sheet) still runs sees none of its tasks and would fail running uploads.
    public func recoverInterrupted(
        now: Date = .now, grace: TimeInterval = ShareSessionCoordinator.grace, reattachingSessions: Bool = true
    ) async {
        let before = now.addingTimeInterval(-grace)
        let stale = store.all().filter { $0.status == .pending && $0.updatedAt < before }
        if stale.contains(where: { $0.sessionIdentifier == nil }) {
            for record in store.markInterrupted(sessionIdentifier: nil, live: [], before: before) {
                onChange?(record)
            }
        }
        guard reattachingSessions else { return }
        for identifier in Set(stale.compactMap(\.sessionIdentifier)) {
            let uploader = uploader(for: identifier)
            let running = await uploader.reconcile(interruptedBefore: before)
            if running == 0, completions[identifier] == nil {
                release(identifier)
            }
        }
    }

    private func uploader(for identifier: String) -> BackgroundUploader {
        if let uploader = uploaders[identifier] {
            return uploader
        }
        let uploader = attach(identifier)
        uploader.onChange = { record in
            Task { @MainActor [weak self] in self?.onChange?(record) }
        }
        uploaders[identifier] = uploader
        return uploader
    }

    private func finishEvents(for identifier: String) {
        release(identifier)
        completions.removeValue(forKey: identifier)?()
    }

    private func release(_ identifier: String) {
        uploaders.removeValue(forKey: identifier)?.invalidate()
    }
}
