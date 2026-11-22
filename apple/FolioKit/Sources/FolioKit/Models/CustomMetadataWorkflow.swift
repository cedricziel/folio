import Foundation

public struct CustomMetadataWorkflowTrigger: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let supportsValues: Bool

    public init(id: String, label: String, supportsValues: Bool) {
        self.id = id
        self.label = label
        self.supportsValues = supportsValues
    }
}

public enum CustomMetadataChangeState: String, CaseIterable, Identifiable, Sendable {
    case set, replaced, cleared
    public var id: Self { self }

    public var label: String {
        switch self {
        case .set: "Set for the first time"
        case .replaced: "Value replaced"
        case .cleared: "Value cleared"
        }
    }
}

public struct CustomMetadataWorkflowDraft: Sendable {
    public var name: String
    public var trigger: String
    public var fieldID: UUID?
    public var kind: CustomFieldKind?
    public var state: CustomMetadataChangeState?
    public var before: CustomFieldValue?
    public var after: CustomFieldValue?
    public var webhook: String
    public var enabled: Bool

    public init(
        name: String, trigger: String, fieldID: UUID? = nil, kind: CustomFieldKind? = nil,
        state: CustomMetadataChangeState? = nil, before: CustomFieldValue? = nil,
        after: CustomFieldValue? = nil, webhook: String, enabled: Bool
    ) {
        self.name = name
        self.trigger = trigger
        self.fieldID = fieldID
        self.kind = kind
        self.state = state
        self.before = before
        self.after = after
        self.webhook = webhook
        self.enabled = enabled
    }
}

public protocol CustomMetadataWorkflowService: Sendable {
    func customMetadataWorkflowTriggers() async throws -> [CustomMetadataWorkflowTrigger]
    func createCustomMetadataWorkflow(_ draft: CustomMetadataWorkflowDraft) async throws
}
