import Foundation

public struct WorkflowTriggerChoice: Identifiable, Equatable, Sendable {
    public enum Target: String, Sendable {
        case document
        case event
        case ingestCandidate = "ingest_candidate"
    }

    public struct ConditionField: Identifiable, Equatable, Sendable {
        public let id: String
        public let title: String
        public let valueType: String

        public init(id: String, title: String, valueType: String) {
            self.id = id
            self.title = title
            self.valueType = valueType
        }
    }

    public let id: String
    public let title: String
    public let target: Target
    public let conditionFields: [ConditionField]

    public init(id: String, title: String, target: Target, conditionFields: [ConditionField]) {
        self.id = id
        self.title = title
        self.target = target
        self.conditionFields = conditionFields
    }
}

public protocol WorkflowTriggerEditingService: Sendable {
    func workflowTriggerChoices(vaultID: UUID) async throws -> [WorkflowTriggerChoice]
    func changeWorkflowTrigger(_ workflow: WorkflowConfigurationSummary, to triggerID: String) async throws
        -> WorkflowConfigurationSummary
}
