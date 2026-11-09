import Foundation

/// Hands user-started work to the system so it can finish in the background with progress the user can cancel.
@MainActor public protocol ContinuedWorkScheduling {
    /// Returns nil when the system won't run the work in the background; the work then runs only in the foreground.
    func begin(title: String, total: Int, onCancel: @escaping @MainActor @Sendable () -> Void) -> (any ContinuedWork)?
}

/// One piece of user-started work the system is showing progress for.
@MainActor public protocol ContinuedWork: AnyObject {
    func update(completed: Int)
    func finish(success: Bool)
}

/// Runs work in the foreground only, as on systems without continued processing.
public struct ForegroundOnlyWork: ContinuedWorkScheduling {
    public init() {}

    public func begin(
        title _: String, total _: Int, onCancel _: @escaping @MainActor @Sendable () -> Void
    ) -> (any ContinuedWork)? {
        nil
    }
}
