import Foundation
import Observation

/// Restores trashed documents through a window's undo manager using the list's scoped service.
@MainActor @Observable
public final class DocumentTrashHistory {
    public var isWorking: Bool { pendingOperations > 0 }
    private let list: DocumentListViewModel
    private var pendingOperations = 0
    @ObservationIgnored private weak var undoManager: UndoManager?
    @ObservationIgnored private var operation: Task<Bool, Never>?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var operationID = 0

    public init(list: DocumentListViewModel) {
        self.list = list
    }

    /// Registers undo after the server confirms deletion; failures appear in the list's action error.
    @discardableResult
    public func moveToTrash(_ document: Document, undoManager: UndoManager?) async -> Bool {
        self.undoManager = undoManager
        let ticket = generation
        let success = await enqueue { await self.list.delete(document) }.value
        if success, ticket == generation {
            register(document, restoring: true)
        }
        return success
    }

    /// Clears this controller's actions and skips queued work when its workspace disappears.
    public func invalidate() {
        generation += 1
        undoManager?.removeAllActions(withTarget: self)
    }

    func waitForPendingOperations() async {
        _ = await operation?.value
    }

    private func apply(_ document: Document, restoring: Bool) {
        // UndoManager records redo only while its synchronous undo handler is running.
        register(document, restoring: !restoring)
        _ = enqueue {
            if restoring {
                return await self.list.restore(document)
            }
            return await self.list.delete(document)
        }
    }

    private func register(_ document: Document, restoring: Bool) {
        guard let undoManager else { return }
        let needsGroup = undoManager.groupingLevel == 0
        if needsGroup { undoManager.beginUndoGrouping() }
        undoManager.registerUndo(withTarget: self) { target in
            target.apply(document, restoring: restoring)
        }
        undoManager.setActionName("Move to Trash")
        if needsGroup { undoManager.endUndoGrouping() }
    }

    private func enqueue(_ action: @escaping @MainActor () async -> Bool) -> Task<Bool, Never> {
        let previous = operation
        let ticket = generation
        operationID += 1
        let id = operationID
        pendingOperations += 1
        let task = Task { @MainActor in
            if let previous { _ = await previous.value }
            defer {
                pendingOperations -= 1
                if id == operationID { operation = nil }
            }
            guard ticket == generation else { return false }
            let success = await action()
            if !success { invalidate() }
            return success
        }
        operation = task
        return task
    }
}
