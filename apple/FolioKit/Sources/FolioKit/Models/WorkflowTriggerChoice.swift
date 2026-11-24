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
    public let canEditDocumentConditions: Bool

    public init(
        id: String, title: String, target: Target, conditionFields: [ConditionField],
        canEditDocumentConditions: Bool = false
    ) {
        self.id = id
        self.title = title
        self.target = target
        self.conditionFields = conditionFields
        self.canEditDocumentConditions = canEditDocumentConditions
    }

    var requiresSingleSelection: Bool {
        target == .ingestCandidate || id == "document.date_offset" || WorkflowCadenceDraft.Trigger(rawValue: id) != nil
    }

    private var documentConditionFamily: String {
        switch id {
        case "document.reviewed", "document.unreviewed": "review"
        case "document.revision_created", "document.revision_restored": "revision"
        default: id
        }
    }

    static func matching(_ ids: [String], in catalog: [Self]) -> Self? {
        let selected = ids.compactMap { id in catalog.first { $0.id == id } }
        guard let primary = selected.first, selected.count == ids.count, Set(ids).count == ids.count,
            selected.count == 1 || !selected.contains(where: \.requiresSingleSelection)
        else { return nil }
        let target: Target = selected.allSatisfy { $0.target == primary.target } ? primary.target : .event
        let fields = primary.conditionFields.filter { field in
            selected.dropFirst().allSatisfy { choice in
                (!field.id.hasPrefix("event.document_change.")
                    || choice.documentConditionFamily == primary.documentConditionFamily)
                    && choice.conditionFields.contains { $0.id == field.id && $0.valueType == field.valueType }
            }
        }
        return .init(
            id: primary.id, title: primary.title, target: target, conditionFields: fields,
            canEditDocumentConditions: selected.allSatisfy(\.canEditDocumentConditions))
    }
}

public protocol WorkflowTriggerEditingService: Sendable {
    func workflowTriggerChoices(vaultID: UUID) async throws -> [WorkflowTriggerChoice]
    func workflowConfiguration(vaultID: UUID, workflowID: UUID) async throws -> WorkflowConfigurationSummary
    func changeWorkflowTrigger(_ workflow: WorkflowConfigurationSummary, to triggerID: String) async throws
        -> WorkflowConfigurationSummary
}
