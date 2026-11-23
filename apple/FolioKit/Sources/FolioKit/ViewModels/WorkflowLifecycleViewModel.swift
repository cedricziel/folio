import Foundation
import Observation

@MainActor @Observable
public final class WorkflowLifecycleViewModel {
    public private(set) var workflows: [WorkflowConfigurationSummary] = []
    public private(set) var deleted = false
    public private(set) var isLoading = false
    public private(set) var hasLoaded = false
    public private(set) var isMutating = false
    public private(set) var error: String?
    public private(set) var mutationError: String?
    public private(set) var accessDenied = false
    public let canManage: Bool
    public var canRead: Bool { canManage || service is any WorkflowMemberReadService }
    private let vaultID: UUID
    private let service: any WorkflowLifecycleService
    private var requestID = UUID()
    private var triggerTitles: [String: String] = [:]

    public init(vaultID: UUID, service: any WorkflowLifecycleService, canManage: Bool) {
        self.vaultID = vaultID
        self.service = service
        self.canManage = canManage
    }

    public func load(deleted: Bool? = nil) async {
        guard canRead, !isMutating, canManage || deleted != true else { return }
        if let deleted, self.deleted != deleted {
            self.deleted = deleted
            workflows = []
            hasLoaded = false
        }
        let request = UUID()
        requestID = request
        let showDeleted = self.deleted
        isLoading = true
        error = nil
        mutationError = nil
        defer {
            if requestID == request {
                isLoading = false
            }
        }
        do {
            let rows: [WorkflowConfigurationSummary]
            if canManage {
                rows = try await service.workflowConfigurations(vaultID: vaultID, deleted: showDeleted)
            } else if let memberRead = service as? any WorkflowMemberReadService {
                rows = try await memberRead.memberWorkflowConfigurations(vaultID: vaultID)
            } else {
                return
            }
            guard requestID == request, !Task.isCancelled else { return }
            guard rows.allSatisfy({ $0.vaultID == vaultID && ($0.deletedAt != nil) == showDeleted }) else {
                throw FolioError.unreadableResponse
            }
            workflows = rows
            hasLoaded = true
            accessDenied = false
        } catch {
            guard requestID == request, !Task.isCancelled else { return }
            if Self.isForbidden(error) || (!canManage && Self.isUnavailableToMember(error)) {
                workflows = []
                accessDenied = true
            }
            self.error = Self.message(
                error, fallback: "Couldn't load workflows. Try again.", canManage: canManage)
        }
    }

    public func moveToTrash(_ workflow: WorkflowConfigurationSummary) async -> Bool {
        await mutate(workflow, restoring: false)
    }

    public func restore(_ workflow: WorkflowConfigurationSummary) async -> Bool {
        await mutate(workflow, restoring: true)
    }

    func triggerTitle(for workflow: WorkflowConfigurationSummary) -> String {
        workflow.selectedTriggers.map { id in
            triggerTitles[id] ?? (id == workflow.trigger ? workflow.triggerTitle : id)
        }.joined(separator: " · ")
    }

    func applyTriggerUpdate(_ workflow: WorkflowConfigurationSummary, choice: WorkflowTriggerChoice?)
        -> WorkflowConfigurationSummary?
    {
        guard canManage, !accessDenied, !deleted, workflow.vaultID == vaultID, workflow.deletedAt == nil,
            let index = workflows.firstIndex(where: { $0.id == workflow.id })
        else { return nil }
        let current = workflows[index]
        if let revision = current.revision, (workflow.revision ?? 0) < revision {
            return current
        }
        requestID = UUID()
        isLoading = false
        if let choice { triggerTitles[choice.id] = choice.title }
        workflows[index] = workflow
        return workflow
    }

    public func dismissMutationError() {
        mutationError = nil
    }

    private func mutate(_ workflow: WorkflowConfigurationSummary, restoring: Bool) async -> Bool {
        guard canManage, !accessDenied, !isMutating, workflow.vaultID == vaultID,
            (workflow.deletedAt != nil) == restoring,
            workflows.contains(where: { $0.id == workflow.id })
        else { return false }
        requestID = UUID()
        isLoading = false
        isMutating = true
        mutationError = nil
        defer { isMutating = false }
        do {
            if restoring {
                let restored = try await service.restoreWorkflow(vaultID: vaultID, workflowID: workflow.id)
                guard restored.vaultID == vaultID, restored.id == workflow.id, restored.deletedAt == nil else {
                    throw FolioError.unreadableResponse
                }
            } else {
                try await service.deleteWorkflow(vaultID: vaultID, workflowID: workflow.id)
            }
            workflows.removeAll { $0.id == workflow.id }
            return true
        } catch {
            if Self.isForbidden(error) {
                workflows = []
                accessDenied = true
            }
            mutationError = Self.message(
                error,
                fallback: restoring
                    ? "Couldn't restore this workflow. Try again." : "Couldn't move this workflow to Trash. Try again."
            )
            return false
        }
    }

    static func isForbidden(_ error: Error) -> Bool {
        if case FolioError.server(status: 403, message: _) = error {
            return true
        }
        return false
    }

    static func isUnavailableToMember(_ error: Error) -> Bool {
        switch error as? FolioError {
        case .server(status: 404, message: _), .signInRequired: true
        default: false
        }
    }

    static func message(_ error: Error, fallback: String, canManage: Bool = true) -> String {
        if let error = error as? FolioError, error == .signInRequired {
            return "Sign in again to continue."
        }
        if !canManage && (isForbidden(error) || isUnavailableToMember(error)) {
            return "Workflow summaries aren't available for this vault."
        }
        if isForbidden(error) {
            return "Only vault owners can manage workflows."
        }
        return fallback
    }
}

@MainActor @Observable
public final class WorkflowHistoryViewModel {
    public private(set) var runs: [WorkflowExecutionSummary] = []
    public private(set) var isLoading = false
    public private(set) var hasLoaded = false
    public private(set) var error: String?
    public private(set) var accessDenied = false
    private let vaultID: UUID
    private let workflowID: UUID
    private let service: any WorkflowLifecycleService
    private let canManage: Bool
    private var requestID = UUID()

    public init(vaultID: UUID, workflowID: UUID, service: any WorkflowLifecycleService, canManage: Bool) {
        self.vaultID = vaultID
        self.workflowID = workflowID
        self.service = service
        self.canManage = canManage
    }

    public func load() async {
        let memberRead = service as? any WorkflowMemberReadService
        guard canManage || memberRead != nil else { return }
        let request = UUID()
        requestID = request
        isLoading = true
        error = nil
        defer {
            if requestID == request {
                isLoading = false
            }
        }
        do {
            let rows: [WorkflowExecutionSummary]
            if canManage {
                rows = try await service.workflowHistory(vaultID: vaultID, workflowID: workflowID)
            } else if let memberRead {
                rows = try await memberRead.memberWorkflowHistory(vaultID: vaultID, workflowID: workflowID)
            } else {
                return
            }
            guard requestID == request, !Task.isCancelled else { return }
            guard rows.allSatisfy({ $0.vaultID == vaultID && $0.workflowID == workflowID }) else {
                throw FolioError.unreadableResponse
            }
            runs = rows
            hasLoaded = true
            accessDenied = false
        } catch {
            guard requestID == request, !Task.isCancelled else { return }
            if WorkflowLifecycleViewModel.isForbidden(error)
                || (!canManage && WorkflowLifecycleViewModel.isUnavailableToMember(error))
            {
                runs = []
                accessDenied = true
            }
            self.error = WorkflowLifecycleViewModel.message(
                error, fallback: "Couldn't load run history. Try again.", canManage: canManage)
        }
    }
}
