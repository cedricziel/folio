import FolioKit
import Foundation

#if os(iOS)
    import BackgroundTasks
#endif

/// Continues user-started work after Folio leaves the foreground, with system progress the user can cancel.
/// Uses continued-processing tasks on iOS 26; elsewhere the work runs only in the foreground, as before.
struct SystemContinuedWork: ContinuedWorkScheduling {
    func begin(title: String, total: Int, onCancel: @escaping @MainActor @Sendable () -> Void) -> (any ContinuedWork)? {
        #if os(iOS)
            if #available(iOS 26, *) {
                return ContinuedProcessingWork.submit(title: title, total: total, onCancel: onCancel)
            }
        #endif
        return nil
    }
}

#if os(iOS)
    /// Each request registers its own identifier under the permitted `<bundle id>.continued.*` pattern.
    @available(iOS 26, *)
    @MainActor private final class ContinuedProcessingWork: ContinuedWork {
        private let title: String
        private let total: Int
        private var completed = 0
        private var result: Bool?
        private var task: BGContinuedProcessingTask?

        private init(title: String, total: Int) {
            self.title = title
            self.total = total
        }

        static func submit(
            title: String, total: Int, onCancel: @escaping @MainActor @Sendable () -> Void
        ) -> ContinuedProcessingWork? {
            guard let bundleID = Bundle.main.bundleIdentifier, permitsContinuedTasks(bundleID) else { return nil }
            let identifier = "\(bundleID).continued.\(UUID().uuidString)"
            let work = ContinuedProcessingWork(title: title, total: total)
            // Handlers can't be unregistered, so this one holds the batch weakly: once the batch ends, only the
            // closure and its identifier stay registered.
            let registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) {
                [weak work] task in
                MainActor.assumeIsolated {
                    guard let task = task as? BGContinuedProcessingTask else {
                        task.setTaskCompleted(success: false)
                        return
                    }
                    guard let work else {
                        // The batch ended before the system started the task, and its own UI showed the outcome.
                        task.setTaskCompleted(success: true)
                        return
                    }
                    work.attach(task, onCancel: onCancel)
                }
            }
            guard registered else { return nil }
            let request = BGContinuedProcessingTaskRequest(
                identifier: identifier, title: title, subtitle: work.subtitle)
            request.strategy = .fail
            do {
                try BGTaskScheduler.shared.submit(request)
            } catch {
                return nil
            }
            return work
        }

        /// Registering an identifier Info.plist doesn't permit raises; builds without the entry stay foreground-only.
        private static func permitsContinuedTasks(_ bundleID: String) -> Bool {
            let permitted = Bundle.main.object(forInfoDictionaryKey: "BGTaskSchedulerPermittedIdentifiers") as? [String]
            return permitted?.contains("\(bundleID).continued.*") == true
        }

        func update(completed: Int) {
            self.completed = completed
            report()
        }

        func finish(success: Bool) {
            guard result == nil else { return }
            result = success
            report()
            task?.setTaskCompleted(success: success)
        }

        private func attach(_ task: BGContinuedProcessingTask, onCancel: @escaping @MainActor @Sendable () -> Void) {
            // Expiration covers both the system reclaiming time and the person cancelling from the progress UI.
            task.expirationHandler = {
                Task { @MainActor in onCancel() }
            }
            task.progress.totalUnitCount = Int64(total)
            self.task = task
            report()
            if let result {
                task.setTaskCompleted(success: result)
            }
        }

        private var subtitle: String {
            "\(completed) of \(total) documents"
        }

        private func report() {
            guard let task else { return }
            task.progress.completedUnitCount = Int64(completed)
            task.updateTitle(title, subtitle: subtitle)
        }
    }
#endif
