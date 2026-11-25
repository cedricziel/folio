import FolioAPI
import Foundation

/// Lifecycle metadata for a workflow, excluding its conditions, actions and stored credentials.
public struct WorkflowConfigurationSummary: Identifiable, Decodable, Sendable, Equatable {
    public let id: UUID
    public let vaultID: UUID
    public let name: String
    public let enabled: Bool
    public let trigger: String
    public let triggers: [String]?
    public let cadence: Components.Schemas.WorkflowCadence?
    public let actionCount: Int?
    let triggerLabels: [String]?
    public let createdAt: Date
    public let updatedAt: Date
    public let revision: Int64?
    public let deletedAt: Date?
    public let createdBy: String?
    public let updatedBy: String?

    enum CodingKeys: String, CodingKey {
        case id, name, enabled, trigger, triggers, revision, cadence
        case vaultID = "vault_id"
        case actionCount = "action_count"
        case triggerLabels = "trigger_labels"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
        case deletedAt = "deleted_at"
        case createdBy = "created_by"
        case updatedBy = "updated_by"
    }

    public init(
        id: UUID, vaultID: UUID, name: String, enabled: Bool, trigger: String,
        createdAt: Date, updatedAt: Date, deletedAt: Date? = nil,
        createdBy: String? = nil, updatedBy: String? = nil, revision: Int64? = nil,
        cadence: Components.Schemas.WorkflowCadence? = nil, triggers: [String]? = nil, actionCount: Int? = nil,
        triggerLabels: [String]? = nil
    ) {
        self.id = id
        self.vaultID = vaultID
        self.name = name
        self.enabled = enabled
        self.trigger = trigger
        self.triggers = triggers
        self.cadence = cadence
        self.actionCount = actionCount
        self.triggerLabels = triggerLabels
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.revision = revision
        self.deletedAt = deletedAt
        self.createdBy = createdBy
        self.updatedBy = updatedBy
    }

    public var stateTitle: String {
        deletedAt != nil ? "In Trash" : enabled ? "On" : "Off"
    }

    public var selectedTriggers: [String] {
        guard let triggers, !triggers.isEmpty else { return [trigger] }
        return triggers
    }

    public var triggerTitle: String {
        Self.title(forTrigger: trigger)
    }

    static func title(forTrigger trigger: String) -> String {
        switch trigger {
        case "document.uploaded": "When a document is added"
        case "document.processed": "When a document has been read"
        case "document.updated": "When a document's details change"
        case "document.title_changed": "When a document title changes"
        case "document.tags_changed": "When document tags change"
        case "document.correspondent_changed": "When a document correspondent changes"
        case "document.document_type_changed": "When a document type changes"
        case "document.date_changed": "When a document date changes"
        case "document.reviewed": "When a document is marked reviewed"
        case "document.unreviewed": "When a document is marked unreviewed"
        case "document.revision_created": "When a document file revision is created"
        case "document.revision_restored": "When a document file revision is restored"
        case "document.deleted": "When a document is deleted"
        case "ingest.received": "When a file arrives, before import"
        case "ingest.imported": "Item imported"
        case "ingest.skipped": "Item skipped by a rule"
        case "ingest.duplicate": "Duplicate item found"
        case "ingest.failed": "Item import failed"
        case "source.failed": "Source became unavailable"
        case "source.recovered": "Source recovered"
        case "workflow.scheduled": "On a schedule"
        case "workflow.created": "When a workflow is created"
        case "workflow.updated": "When a workflow changes"
        case "workflow.enabled": "When a workflow is enabled"
        case "workflow.disabled": "When a workflow is disabled"
        case "workflow.deleted": "When a workflow moves to Trash"
        case "workflow.restored": "When a workflow is restored"
        case "workflow_run.succeeded": "When a workflow run succeeds"
        case "workflow_run.failed": "When a workflow run fails"
        case "document.date_offset": "Relative to the document date"
        case "document.metadata.changed": "When custom field values change"
        case "metadata_field.created": "When a custom field is created"
        case "metadata_field.updated": "When a custom field changes"
        case "metadata_field.deleted": "When a custom field is retired"
        case "metadata_field.restored": "When a custom field is restored"
        case "metadata_option.created": "When a custom field option is created"
        case "metadata_option.updated": "When a custom field option changes"
        case "metadata_option.deleted": "When a custom field option is retired"
        case "metadata_option.restored": "When a custom field option is restored"
        case "saved_view.created": "When a shared saved view is created"
        case "saved_view.updated": "When a shared saved view is updated"
        case "saved_view.deleted": "When a shared saved view is deleted"
        case "tag.created": "When a tag is created"
        case "tag.updated": "When a tag is updated"
        case "tag.deleted": "When a tag is deleted"
        case "tag.restored": "When a tag is restored"
        case "correspondent.created": "When a correspondent is created"
        case "correspondent.updated": "When a correspondent is updated"
        case "correspondent.deleted": "When a correspondent is deleted"
        case "correspondent.restored": "When a correspondent is restored"
        case "document_type.created": "When a document type is created"
        case "document_type.updated": "When a document type is updated"
        case "document_type.deleted": "When a document type is deleted"
        case "document_type.restored": "When a document type is restored"
        case "source.created": "When a managed source is created"
        case "source.updated": "When a managed source is updated"
        case "source.deleted": "When a managed source is deleted"
        case "source.restored": "When a managed source is restored"
        case "vault.updated": "When this vault is renamed"
        case "vault.restored": "When this vault is restored"
        case "vault_membership.granted": "When a vault membership is granted"
        case "vault_membership.role_changed": "When a vault member’s role changes"
        case "vault_membership.revoked": "When a vault membership is revoked"
        default: trigger
        }
    }
}

/// A retained execution outcome. Runs can outlive their workflow's active configuration.
public struct WorkflowExecutionSummary: Identifiable, Decodable, Sendable, Equatable {
    public let id: UUID
    public let vaultID: UUID
    public let workflowID: UUID
    public let event: String
    public let documentID: UUID?
    public let status: String
    public let attempts: Int
    public let completedActions: Int
    public let lastError: String?
    public let responseStatus: Int?
    public let createdAt: Date
    public let finishedAt: Date?

    enum CodingKeys: String, CodingKey {
        case id, event, status, attempts
        case vaultID = "vault_id"
        case workflowID = "workflow_id"
        case documentID = "document_id"
        case completedActions = "completed_actions"
        case lastError = "last_error"
        case responseStatus = "response_status"
        case createdAt = "created_at"
        case finishedAt = "finished_at"
    }

    public init(
        id: UUID, vaultID: UUID, workflowID: UUID, event: String, documentID: UUID? = nil,
        status: String, attempts: Int, completedActions: Int, lastError: String? = nil,
        responseStatus: Int? = nil, createdAt: Date, finishedAt: Date? = nil
    ) {
        self.id = id
        self.vaultID = vaultID
        self.workflowID = workflowID
        self.event = event
        self.documentID = documentID
        self.status = status
        self.attempts = attempts
        self.completedActions = completedActions
        self.lastError = lastError
        self.responseStatus = responseStatus
        self.createdAt = createdAt
        self.finishedAt = finishedAt
    }

    public var statusTitle: String {
        switch status {
        case "pending": "Waiting"
        case "succeeded": "Succeeded"
        case "failed": "Failed"
        default: status
        }
    }
}

/// Read-only workflow projections in an explicitly selected vault.
public protocol WorkflowMemberReadService: Sendable {
    func memberWorkflowConfigurations(vaultID: UUID) async throws -> [WorkflowConfigurationSummary]
    func memberWorkflowHistory(vaultID: UUID, workflowID: UUID) async throws -> [WorkflowExecutionSummary]
}

/// Owner-only workflow lifecycle operations in an explicitly selected vault.
public protocol WorkflowLifecycleService: Sendable {
    func workflowConfigurations(vaultID: UUID, deleted: Bool) async throws -> [WorkflowConfigurationSummary]
    func workflowHistory(vaultID: UUID, workflowID: UUID) async throws -> [WorkflowExecutionSummary]
    func deleteWorkflow(vaultID: UUID, workflowID: UUID) async throws
    func restoreWorkflow(vaultID: UUID, workflowID: UUID) async throws -> WorkflowConfigurationSummary
}

protocol WorkflowAuthoringService: WorkflowTriggerEditingService {
    func createWorkflow(
        vaultID: UUID, name: String, choices: [WorkflowTriggerChoice], actions: WorkflowActionDraft,
        cadence: Components.Schemas.WorkflowCadence?
    ) async throws -> WorkflowConfigurationSummary

    func saveWorkflowTriggers(
        _ workflow: WorkflowConfigurationSummary, to triggerIDs: [String], cadence: Components.Schemas.WorkflowCadence?,
        documentConditions: WorkflowDocumentConditionDraft?
    ) async throws -> WorkflowConfigurationSummary

    func saveWorkflowSettings(
        _ workflow: WorkflowConfigurationSummary, name: String, enabled: Bool
    ) async throws -> WorkflowConfigurationSummary
}

func validatedWorkflowName(_ name: String) throws -> String {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
        throw FolioError.server(status: 400, message: "Enter a workflow name.")
    }
    return trimmed
}
