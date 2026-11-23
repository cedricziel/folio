import Foundation

public enum CustomFieldKind: String, CaseIterable, Identifiable, Sendable {
    case text
    case longText = "long_text"
    case boolean
    case integer
    case decimal
    case date
    case money
    case select
    case url
    case documentLink = "document_link"

    public var id: Self { self }

    public var label: String {
        switch self {
        case .text: "Text"
        case .longText: "Long text"
        case .boolean: "Yes or no"
        case .integer: "Whole number"
        case .decimal: "Decimal"
        case .date: "Date"
        case .money: "Money"
        case .select: "Select"
        case .url: "URL"
        case .documentLink: "Document link"
        }
    }
}

public enum CustomFieldValue: Hashable, Sendable {
    case text(String)
    case longText(String)
    case boolean(Bool)
    case integer(String)
    case decimal(String)
    case date(String)
    case money(amount: String, currency: String)
    case select(UUID)
    case url(String)
    case documentLink(UUID)
}

public struct CustomFieldDraft: Equatable, Sendable {
    public var name: String
    public var kind: CustomFieldKind
    public var required: Bool
    public var unique: Bool

    public init(name: String = "", kind: CustomFieldKind = .text, required: Bool = false, unique: Bool = false) {
        self.name = name
        self.kind = kind
        self.required = required
        self.unique = unique
    }
}

public struct CustomFieldDefinition: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let draft: CustomFieldDraft
    public let isRetired: Bool

    public init(id: UUID, draft: CustomFieldDraft, isRetired: Bool) {
        self.id = id
        self.draft = draft
        self.isRetired = isRetired
    }
}

public struct CustomFieldOption: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let label: String
    public let position: UInt32
    public let isRetired: Bool

    public init(id: UUID, label: String, position: UInt32, isRetired: Bool) {
        self.id = id
        self.label = label
        self.position = position
        self.isRetired = isRetired
    }
}

public struct CustomFieldLink: Equatable, Sendable {
    public let title: String
    public let isTrashed: Bool

    public init(title: String, isTrashed: Bool) {
        self.title = title
        self.isTrashed = isTrashed
    }
}

public struct CustomFieldAssignment: Equatable, Sendable {
    public let fieldID: UUID
    public let value: CustomFieldValue?
    public let link: CustomFieldLink?

    public init(fieldID: UUID, value: CustomFieldValue?, link: CustomFieldLink? = nil) {
        self.fieldID = fieldID
        self.value = value
        self.link = link
    }
}

public struct CustomMetadataSnapshot: Equatable, Sendable {
    public let revision: UUID?
    public let values: [CustomFieldAssignment]

    public init(revision: UUID?, values: [CustomFieldAssignment]) {
        self.revision = revision
        self.values = values
    }
}

public protocol CustomMetadataService: Sendable {
    func customFields(after: UUID?) async throws -> [CustomFieldDefinition]
    func saveCustomField(id: UUID?, draft: CustomFieldDraft) async throws -> CustomFieldDefinition
    func setCustomFieldRetired(id: UUID, retired: Bool) async throws
    func customOption(fieldID: UUID, id: UUID) async throws -> CustomFieldOption
    func customOptions(fieldID: UUID, after: UUID?) async throws -> [CustomFieldOption]
    func saveCustomOption(fieldID: UUID, id: UUID?, label: String, position: UInt32) async throws -> CustomFieldOption
    func setCustomOptionRetired(fieldID: UUID, id: UUID, retired: Bool) async throws
    func customMetadata(documentID: UUID) async throws -> CustomMetadataSnapshot
    func setCustomValue(documentID: UUID, revision: UUID?, fieldID: UUID, value: CustomFieldValue?) async throws
        -> CustomMetadataSnapshot
}
