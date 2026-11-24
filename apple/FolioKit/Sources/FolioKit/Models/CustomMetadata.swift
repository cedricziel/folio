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

    var kind: CustomFieldKind {
        switch self {
        case .text: .text
        case .longText: .longText
        case .boolean: .boolean
        case .integer: .integer
        case .decimal: .decimal
        case .date: .date
        case .money: .money
        case .select: .select
        case .url: .url
        case .documentLink: .documentLink
        }
    }
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

public struct CustomFieldValueDraft: Sendable {
    var text = ""
    var currency = ""
    var boolean: Bool?
    var date: Date?
    var reference: UUID?

    public init(_ value: CustomFieldValue? = nil) {
        switch value {
        case let .text(value), let .longText(value), let .integer(value), let .decimal(value), let .url(value):
            text = value
        case let .date(value):
            date = DocumentDate(rawValue: value)?.start(in: Self.calendar.timeZone)
        case let .boolean(value): boolean = value
        case let .money(amount, code):
            text = amount
            currency = code
        case let .select(id), let .documentLink(id): reference = id
        case nil: break
        }
    }

    public func value(for kind: CustomFieldKind) -> CustomFieldValue? {
        switch kind {
        case .text: .text(text)
        case .longText: .longText(text)
        case .boolean: boolean.map(CustomFieldValue.boolean)
        case .integer: .integer(text)
        case .decimal: .decimal(text)
        case .date:
            date.map { .date($0.formatted(.iso8601.year().month().day().dateSeparator(.dash))) }
        case .money: .money(amount: text, currency: currency)
        case .select: reference.map(CustomFieldValue.select)
        case .url: .url(text)
        case .documentLink: reference.map(CustomFieldValue.documentLink)
        }
    }

    static let calendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        return calendar
    }()
}
