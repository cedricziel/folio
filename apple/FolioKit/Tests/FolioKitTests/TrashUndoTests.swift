import Foundation
import Testing

@testable import FolioKit

@MainActor struct DocumentTrashHistoryTests {
    @Test func movingToTrashCanBeUndoneAndRedone() async {
        let document = Document(title: "Letter", content: "Read", ocrStatus: .done, embedStatus: .done)
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false

        #expect(await history.moveToTrash(document, undoManager: manager))
        #expect(list.rows.isEmpty)
        #expect(list.rowChangeRevision == 1)
        #expect(manager.undoActionName == "Move to Trash")

        manager.undo()
        await history.waitForPendingOperations()
        #expect(list.rows.map(\.id) == [document.id])
        #expect(list.rowChangeRevision == 2)
        #expect(manager.canRedo)

        manager.redo()
        await history.waitForPendingOperations()
        #expect(list.rows.isEmpty)
        #expect(list.rowChangeRevision == 3)
        #expect(manager.canUndo)
    }

    @Test func failedDeletionDoesNotAddAnUndoAction() async {
        let document = Document(title: "Letter")
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        await service.setFailure(.unreachable)

        #expect(await history.moveToTrash(document, undoManager: manager) == false)
        #expect(list.rows.map(\.id) == [document.id])
        #expect(manager.canUndo == false)
        #expect(list.rowChangeRevision == 0)
        #expect(list.actionError != nil)
    }

    @Test func failedRestoreKeepsTheErrorAndClearsInvalidHistory() async {
        let document = Document(title: "Letter")
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        #expect(await history.moveToTrash(document, undoManager: manager))
        await service.setFailure(.unreachable)

        manager.undo()
        await history.waitForPendingOperations()

        #expect(list.rows.isEmpty)
        #expect(list.rowChangeRevision == 1)
        #expect(list.actionError != nil)
        #expect(manager.canRedo == false)
        #expect(manager.canUndo == false)
    }

    @Test func immediateUndoThenRedoPreservesTheRequestedOrder() async {
        let document = Document(title: "Letter", content: "Read", ocrStatus: .done, embedStatus: .done)
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        #expect(await history.moveToTrash(document, undoManager: manager))

        manager.undo()
        manager.redo()
        await history.waitForPendingOperations()

        #expect(list.rows.isEmpty)
        #expect(manager.canUndo)
        #expect(history.isWorking == false)
        #expect(await service.documentList.isEmpty)
    }

    @Test func restoringRespectsTheCurrentInboxFilter() async {
        let document = Document(
            title: "Filed letter", content: "Read", documentTypeID: UUID(), ocrStatus: .done, embedStatus: .done
        )
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        #expect(await history.moveToTrash(document, undoManager: manager))
        list.scope = .inbox
        let incoming = Document(title: "New letter")
        await service.setDocuments([incoming])

        manager.undo()
        await history.waitForPendingOperations()

        #expect(list.rows.map(\.id) == [incoming.id])
        #expect(list.rowChangeRevision == 1)
        #expect(await service.documentList.map(\.id) == [incoming.id, document.id])
        #expect(manager.canRedo)

        manager.redo()
        await history.waitForPendingOperations()
        #expect(list.rows.map(\.id) == [incoming.id])
        #expect(list.rowChangeRevision == 1)
        #expect(await service.documentList.map(\.id) == [incoming.id])
    }

    @Test func leavingTheWorkspacePreservesUnrelatedUndoActions() async {
        let document = Document(title: "Letter")
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        let note = UndoNote()
        manager.beginUndoGrouping()
        manager.registerUndo(withTarget: note) { $0.wasUndone = true }
        manager.endUndoGrouping()
        #expect(await history.moveToTrash(document, undoManager: manager))

        history.invalidate()
        manager.undo()
        await history.waitForPendingOperations()

        #expect(note.wasUndone)
        #expect(list.rows.isEmpty)
        #expect(manager.canRedo == false)
    }

    @Test func leavingTheWorkspaceSkipsAQueuedUndo() async {
        let document = Document(title: "Letter")
        let service = MockFolioService(documents: [document])
        let list = DocumentListViewModel(service: service)
        await list.load()
        let history = DocumentTrashHistory(list: list)
        let manager = UndoManager()
        manager.groupsByEvent = false
        #expect(await history.moveToTrash(document, undoManager: manager))

        manager.undo()
        history.invalidate()
        await history.waitForPendingOperations()

        #expect(list.rows.isEmpty)
        #expect(await service.documentList.isEmpty)
        #expect(manager.canUndo == false)
        #expect(manager.canRedo == false)
    }
}

@MainActor private final class UndoNote {
    var wasUndone = false
}
