import Foundation
import Testing

@testable import FolioKit

@MainActor struct WorkflowLifecycleTests {
    @Test func trashAndRestorePreserveHistoryAndUpdateTheSelectedList() async throws {
        let fixture = LifecycleFixture()
        let model = fixture.model()
        await model.load()
        #expect(await model.moveToTrash(fixture.workflow))
        #expect(model.workflows.isEmpty)
        await model.load(deleted: true)
        let trashed = try #require(model.workflows.first)
        let history = fixture.history()
        await history.load()
        #expect(history.runs == [fixture.run])
        #expect(await model.restore(trashed))
        #expect(model.workflows.isEmpty)
        await model.load(deleted: false)
        #expect(model.workflows == [fixture.workflow])
        await history.load()
        #expect(history.runs == [fixture.run])
    }

    @Test func nonOwnersCannotReadOrChangeWorkflows() async {
        let fixture = LifecycleFixture()
        let model = fixture.model(canManage: false)
        await model.load()
        #expect(!(await model.moveToTrash(fixture.workflow)))
        #expect(!(await model.restore(fixture.workflow)))
        let history = fixture.history(canManage: false)
        await history.load()
        #expect(fixture.service.requestCount == 0)
        #expect(model.workflows.isEmpty)
        #expect(history.runs.isEmpty)
    }

    @Test func failedDeleteRetainsConfigurationAndRetryMovesItToTrash() async {
        let fixture = LifecycleFixture()
        let model = fixture.model()
        await model.load()
        fixture.service.failure = .unreachable
        #expect(!(await model.moveToTrash(fixture.workflow)))
        #expect(model.workflows == [fixture.workflow])
        #expect(model.mutationError == "Couldn't move this workflow to Trash. Try again.")
        fixture.service.failure = nil
        #expect(await model.moveToTrash(fixture.workflow))
        #expect(model.workflows.isEmpty)
        #expect(model.mutationError == nil)
    }

    @Test func revokedOwnerAccessClearsConfigurationAndBlocksMutation() async {
        let fixture = LifecycleFixture()
        let model = fixture.model()
        await model.load()
        fixture.service.failure = .server(status: 403, message: "Private server detail")
        await model.load()
        #expect(model.workflows.isEmpty)
        #expect(model.accessDenied)
        #expect(model.error == "Only vault owners can manage workflows.")
        let count = fixture.service.requestCount
        #expect(!(await model.moveToTrash(fixture.workflow)))
        #expect(fixture.service.requestCount == count)
    }

    @Test func anotherVaultCannotPopulateConfigurationsOrHistory() async {
        let fixture = LifecycleFixture()
        let wrongVault = UUID()
        let model = WorkflowLifecycleViewModel(vaultID: wrongVault, service: fixture.service, canManage: true)
        let history = WorkflowHistoryViewModel(
            vaultID: wrongVault, workflowID: fixture.workflow.id, service: fixture.service, canManage: true
        )
        await model.load()
        await history.load()
        #expect(model.workflows.isEmpty)
        #expect(model.error != nil)
        #expect(history.runs.isEmpty)
        #expect(history.error != nil)
    }

    @Test func failedHistoryRefreshKeepsLoadedRunsAndRetryClearsTheError() async {
        let fixture = LifecycleFixture()
        let history = fixture.history()
        await history.load()
        fixture.service.failure = .unreachable
        await history.load()
        #expect(history.runs == [fixture.run])
        #expect(history.error == "Couldn't load run history. Try again.")
        fixture.service.failure = nil
        await history.load()
        #expect(history.error == nil)
    }

    @Test func aLateTriggerSaveDoesNotReplaceTheNewerRefreshedConfiguration() async {
        let fixture = LifecycleFixture()
        let model = fixture.model()
        let original = fixture.workflow
        func revision(_ value: Int64, enabled: Bool, trigger: String) -> WorkflowConfigurationSummary {
            WorkflowConfigurationSummary(
                id: original.id, vaultID: original.vaultID, name: original.name, enabled: enabled,
                trigger: trigger, createdAt: original.createdAt, updatedAt: original.updatedAt, revision: value
            )
        }
        fixture.service.workflow = revision(1, enabled: true, trigger: original.trigger)
        await model.load()
        let choice = WorkflowTriggerChoice(
            id: "document_note.created", title: "When a note is created", target: .document, conditionFields: []
        )
        let saved = revision(2, enabled: true, trigger: choice.id)
        let disabled = revision(3, enabled: false, trigger: choice.id)
        fixture.service.workflow = disabled
        await model.load()
        #expect(model.applyTriggerUpdate(saved, choice: choice) == disabled)
        #expect(model.workflows == [disabled])

        let latest = revision(4, enabled: false, trigger: choice.id)
        #expect(model.applyTriggerUpdate(latest, choice: choice) == latest)
        #expect(model.workflows == [latest])
    }

    @Test func switchingToTrashIgnoresAnOlderActiveListResponse() async {
        let fixture = LifecycleFixture()
        fixture.service.holdActiveList = true
        let model = fixture.model()
        let activeLoad = Task { await model.load() }
        await fixture.service.waitForHeldList()
        await model.load(deleted: true)
        fixture.service.releaseActiveList()
        await activeLoad.value
        #expect(model.deleted)
        #expect(model.workflows.isEmpty)
        #expect(model.hasLoaded)
        #expect(!model.isLoading)
    }
}

@MainActor private struct LifecycleFixture {
    let workflow: WorkflowConfigurationSummary
    let run: WorkflowExecutionSummary
    let service: LifecycleStore

    init() {
        let vault = UUID()
        workflow = WorkflowConfigurationSummary(
            id: UUID(), vaultID: vault, name: "File invoices", enabled: true,
            trigger: "document.uploaded", createdAt: .distantPast, updatedAt: .distantPast,
            createdBy: "creator", updatedBy: "editor"
        )
        run = WorkflowExecutionSummary(
            id: UUID(), vaultID: vault, workflowID: workflow.id, event: workflow.trigger,
            status: "failed", attempts: 3, completedActions: 1, lastError: "Receiver unavailable",
            responseStatus: 503, createdAt: .distantPast, finishedAt: .distantPast
        )
        service = LifecycleStore(workflow: workflow, run: run)
    }

    func model(canManage: Bool = true) -> WorkflowLifecycleViewModel {
        WorkflowLifecycleViewModel(vaultID: workflow.vaultID, service: service, canManage: canManage)
    }

    func history(canManage: Bool = true) -> WorkflowHistoryViewModel {
        WorkflowHistoryViewModel(
            vaultID: workflow.vaultID, workflowID: workflow.id, service: service, canManage: canManage)
    }
}

@MainActor private final class LifecycleStore: WorkflowLifecycleService {
    var workflow: WorkflowConfigurationSummary
    let run: WorkflowExecutionSummary
    var failure: FolioError?
    var requestCount = 0
    var trashed = false
    var holdActiveList = false
    private var heldList: CheckedContinuation<Void, Never>?
    private var waitingForHeldList: CheckedContinuation<Void, Never>?

    init(workflow: WorkflowConfigurationSummary, run: WorkflowExecutionSummary) {
        self.workflow = workflow
        self.run = run
    }

    func workflowConfigurations(vaultID _: UUID, deleted: Bool) async throws -> [WorkflowConfigurationSummary] {
        requestCount += 1
        if let failure {
            throw failure
        }
        if !deleted, holdActiveList {
            await withCheckedContinuation { continuation in
                heldList = continuation
                waitingForHeldList?.resume()
                waitingForHeldList = nil
            }
        }
        return deleted == trashed ? [current] : []
    }

    func workflowHistory(vaultID _: UUID, workflowID _: UUID) async throws -> [WorkflowExecutionSummary] {
        requestCount += 1
        if let failure {
            throw failure
        }
        return [run]
    }

    func deleteWorkflow(vaultID _: UUID, workflowID _: UUID) async throws {
        requestCount += 1
        if let failure {
            throw failure
        }
        trashed = true
    }

    func restoreWorkflow(vaultID _: UUID, workflowID _: UUID) async throws -> WorkflowConfigurationSummary {
        requestCount += 1
        if let failure {
            throw failure
        }
        trashed = false
        return current
    }

    func waitForHeldList() async {
        if heldList != nil {
            return
        }
        await withCheckedContinuation { waitingForHeldList = $0 }
    }

    func releaseActiveList() {
        heldList?.resume()
        heldList = nil
    }

    private var current: WorkflowConfigurationSummary {
        WorkflowConfigurationSummary(
            id: workflow.id, vaultID: workflow.vaultID, name: workflow.name, enabled: workflow.enabled,
            trigger: workflow.trigger, createdAt: workflow.createdAt, updatedAt: workflow.updatedAt,
            deletedAt: trashed ? .distantPast : nil, createdBy: workflow.createdBy, updatedBy: workflow.updatedBy,
            revision: workflow.revision
        )
    }
}
