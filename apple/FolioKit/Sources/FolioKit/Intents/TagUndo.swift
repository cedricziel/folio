import Foundation

/// Offers a tag change that took effect to the system's Undo, which then puts the same documents back.
@MainActor
public enum TagUndo {
    public typealias Revert = @Sendable (TagChange) async -> Void

    public static func register(_ change: TagChange, with undoManager: UndoManager?, revert: @escaping Revert) {
        guard let undoManager, !change.documents.isEmpty else { return }
        undoManager.registerUndo(withTarget: TagUndoTarget.shared) { _ in
            Task { await revert(change.reverted) }
        }
        undoManager.setActionName(actionName(for: change))
    }

    public nonisolated static func actionName(for change: TagChange) -> String {
        change.kind == .add ? "Add Tag" : "Remove Tag"
    }

    /// Applies the reverted change through the signed-in server. Undo has no place to show an error.
    @Sendable public nonisolated static func revert(_ change: TagChange) async {
        _ = try? await IntentEnvironment.run { try await $0.apply(change) }
    }
}

/// What the undo registration hangs off; the change itself travels in the closure.
private final class TagUndoTarget: Sendable {
    static let shared = TagUndoTarget()
}
