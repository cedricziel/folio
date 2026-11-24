import Foundation
import OpenAPIRuntime

struct WorkflowConditionTree: Identifiable, Sendable {
    typealias Object = [String: (any Sendable)?]

    enum Match: String, CaseIterable, Identifiable, Sendable {
        case all, any, none

        var id: Self {
            self
        }

        var title: String {
            switch self {
            case .all: "Match all"
            case .any: "Match any"
            case .none: "Match none"
            }
        }
    }

    enum Actor: String, CaseIterable, Identifiable, Sendable {
        case any = ""
        case system
        case user

        var id: Self { self }

        var title: String {
            switch self {
            case .any: "Any actor"
            case .system: "Folio"
            case .user: "Specific user"
            }
        }

        static func isAvailable(for choice: WorkflowTriggerChoice) -> Bool {
            ["event.actor.type", "event.actor.user_id"].allSatisfy { id in
                choice.conditionFields.contains { $0.id == id && $0.valueType == "text" }
            }
        }
    }

    enum ProcessingField: String, Sendable {
        case stage, outcome, attempt
        case runID = "run_id"
        case revisionID = "revision_id"
        case reason = "reason_code"
        case retryTime = "retry_at"
        case hasText = "has_text"

        var valueType: String {
            switch self {
            case .stage, .outcome, .reason: "text"
            case .runID, .revisionID: "identifier"
            case .attempt: "integer"
            case .retryTime: "timestamp"
            case .hasText: "boolean"
            }
        }
    }

    struct Field: Identifiable, Sendable {
        let id: String
        let title: String
        var isIdentifier = false
        var processing: ProcessingField?

        var isSourceKind: Bool { id == "source.kind" }

        var isTextMatch: Bool {
            switch id {
            case "source.email_from", "source.email_to", "source.email_subject": true
            default: false
            }
        }

        fileprivate var path: [String] {
            id.split(separator: ".").map(String.init)
        }

        private static let fileFields: [Self] = [
            .init(id: "filename_glob", title: "Filename pattern"),
            .init(id: "mime_type", title: "MIME type"),
            .init(id: "source.kind", title: "Source"),
            .init(id: "source.folder_path_glob", title: "Folder path pattern"),
            .init(id: "source.email_from", title: "Email sender"),
            .init(id: "source.email_to", title: "Email recipient"),
            .init(id: "source.email_subject", title: "Email subject"),
            .init(id: "source.email_attachment_glob", title: "Attachment name pattern"),
            .init(id: "source.email_mailbox_glob", title: "Mailbox pattern"),
        ]

        private static let classificationFields: [Self] = [
            .init(id: "tag_id", title: "Tag ID", isIdentifier: true),
            .init(id: "correspondent_id", title: "Correspondent ID", isIdentifier: true),
            .init(id: "document_type_id", title: "Document type ID", isIdentifier: true),
        ]

        private static let snapshotFields = fileFields + classificationFields

        static func available(for choice: WorkflowTriggerChoice, includeLegacyDefaults: Bool = true) -> [Self] {
            let snapshot = choice.conditionFields.filter { advertised in
                snapshotFields.contains { $0.id == advertised.id }
            }
            var fields =
                snapshot.isEmpty && includeLegacyDefaults
                ? legacySnapshotFields(for: choice.id)
                : snapshot.compactMap { advertised in
                    guard let field = snapshotFields.first(where: { $0.id == advertised.id }),
                        advertised.valueType == (field.isIdentifier ? "identifier" : "text")
                    else { return nil }
                    return Self(id: field.id, title: advertised.title, isIdentifier: field.isIdentifier)
                }
            for fieldID in ["event.subject_id", "event.workflow_id"] {
                if let advertised = choice.conditionFields.first(where: {
                    $0.id == fieldID && $0.valueType == "identifier"
                }) {
                    fields.append(.init(id: advertised.id, title: advertised.title, isIdentifier: true))
                }
            }
            if choice.id.hasPrefix("document.processing.") {
                fields += choice.conditionFields.compactMap { advertised in
                    guard advertised.id.hasPrefix("data."),
                        let processing = ProcessingField(rawValue: String(advertised.id.dropFirst("data.".count))),
                        processing.valueType == advertised.valueType
                    else { return nil }
                    return Self(
                        id: "event.processing.\(processing.rawValue)", title: advertised.title,
                        isIdentifier: processing.valueType == "identifier", processing: processing
                    )
                }
            }
            return fields
        }

        private static func legacySnapshotFields(for triggerID: String) -> [Self] {
            switch triggerID {
            case "ingest.received":
                fileFields
            case "document.uploaded", "document.processed", "document.updated", "document.deleted",
                "document.title_changed", "document.tags_changed", "document.correspondent_changed",
                "document.document_type_changed":
                snapshotFields
            default:
                []
            }
        }
    }

    struct TextMatch: Equatable, Sendable {
        enum Mode: String, CaseIterable, Identifiable, Sendable {
            case contains
            case glob
            case regex

            var id: Self { self }

            var title: String {
                switch self {
                case .contains: "Contains"
                case .glob: "Wildcard"
                case .regex: "Regular expression"
                }
            }
        }

        let mode: Mode
        let pattern: String
    }

    static let maximumDepth = 8
    static let maximumNodes = 64

    let id = UUID()
    private var values: Object = [:]
    private var groups: [Match: [Self]] = [:]

    init() {}

    init(_ payload: OpenAPIObjectContainer) throws {
        var remaining = Self.maximumNodes
        try self.init(payload.value, depth: 0, remaining: &remaining)
    }

    private init(_ object: Object, depth: Int, remaining: inout Int) throws {
        guard depth <= Self.maximumDepth, remaining > 0 else { throw FolioError.unreadableResponse }
        remaining -= 1
        values = object
        for kind in Match.allCases {
            guard let raw = values.removeValue(forKey: kind.rawValue) else { continue }
            if kind == .any, raw == nil {
                continue
            }
            guard let children = raw as? [Object] else { throw FolioError.unreadableResponse }
            groups[kind] = try children.map { try Self($0, depth: depth + 1, remaining: &remaining) }
        }
    }

    subscript(_ kind: Match) -> [Self]? {
        get { groups[kind] }
        set { groups[kind] = newValue }
    }

    subscript(_ field: Field) -> String {
        get {
            let value = Self.value(at: field.path[...], in: values)
            if field.processing == .hasText, let boolean = value as? Bool {
                return String(boolean)
            }
            if field.processing == .attempt, let attempt = value as? Int {
                return String(attempt)
            }
            return value as? String ?? ""
        }
        set {
            let text = field.processing == nil ? newValue : newValue.trimmingCharacters(in: .whitespacesAndNewlines)
            let value: (any Sendable)?
            if text.isEmpty {
                value = nil
            } else if field.processing == .hasText, let boolean = Bool(text) {
                value = boolean
            } else if field.processing == .attempt, let attempt = UInt32(text) {
                value = Int(attempt)
            } else {
                value = text
            }
            Self.set(value, at: field.path[...], in: &values)
        }
    }

    func textMatch(for field: Field) -> TextMatch? {
        guard let comparison = Self.value(at: field.path[...], in: values) as? Object,
            comparison.count == 1,
            let (key, value) = comparison.first,
            let mode = TextMatch.Mode(rawValue: key),
            let pattern = value as? String
        else { return nil }
        return TextMatch(mode: mode, pattern: pattern)
    }

    mutating func setTextMatch(_ pattern: String, mode: TextMatch.Mode, for field: Field) {
        let value: Object? = pattern.isEmpty ? nil : [mode.rawValue: pattern]
        Self.set(value, at: field.path[...], in: &values)
    }

    var actor: Actor {
        get {
            let type = Self.value(at: ["event", "actor", "type"][...], in: values) as? String ?? ""
            return Actor(rawValue: type) ?? .any
        }
        set {
            guard newValue != actor else { return }
            var event = values["event"] as? Object ?? [:]
            if newValue == .any {
                event.removeValue(forKey: "actor")
            } else {
                event["actor"] = ["type": newValue.rawValue]
            }
            if event.isEmpty {
                values.removeValue(forKey: "event")
            } else {
                values["event"] = event
            }
        }
    }

    var actorUserID: String {
        get { self[Self.actorUserField] }
        set { self[Self.actorUserField] = newValue }
    }

    private static let actorUserField = Field(id: "event.actor.user_id", title: "User ID")

    var count: Int {
        1 + groups.values.joined().reduce(0) { $0 + $1.count }
    }

    var depth: Int {
        (groups.values.joined().map(\.depth).max() ?? -1) + 1
    }

    func validationMessage(fields: [Field], supportsActor: Bool = false) -> String? {
        if count > Self.maximumNodes || depth > Self.maximumDepth {
            return "Use at most 64 conditions and eight levels of nesting."
        }
        return invalidFilter(fields: fields, supportsActor: supportsActor)
    }

    private func invalidFilter(fields: [Field], supportsActor: Bool) -> String? {
        if supportsActor && actor == .user && actorUserID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return "Enter the actor's user ID."
        }
        for field in fields {
            let text = self[field]
            guard !text.isEmpty else { continue }
            if field.isIdentifier && UUID(uuidString: text) == nil {
                return "Enter a valid UUID for \(field.title)."
            }
            switch field.processing {
            case .attempt where UInt32(text) == nil:
                return "Enter a valid processing attempt."
            case .hasText where Bool(text) == nil:
                return "Choose whether text was found."
            case .retryTime
            where (try? Date.ISO8601FormatStyle().parse(text)) == nil
                && (try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(text)) == nil:
                return "Enter a valid timestamp for \(field.title)."
            default:
                break
            }
        }
        return Match.allCases.lazy.compactMap { self[$0] }.joined().lazy
            .compactMap { $0.invalidFilter(fields: fields, supportsActor: supportsActor) }.first
    }

    func hasAdditionalFilters(fields: [Field], supportsActor: Bool) -> Bool {
        var additional = self
        if supportsActor { additional.actor = .any }
        for field in fields where !self[field].isEmpty || textMatch(for: field) != nil {
            additional[field] = ""
        }
        return !additional.values.isEmpty
    }

    func payload() throws -> OpenAPIObjectContainer {
        guard count <= Self.maximumNodes, depth <= Self.maximumDepth else {
            throw FolioError.server(status: 400, message: "Use at most 64 conditions and eight levels of nesting.")
        }
        return try OpenAPIObjectContainer(unvalidatedValue: object)
    }

    private var object: Object {
        var object = values
        for (kind, children) in groups where kind == .any || !children.isEmpty {
            object[kind.rawValue] = children.map(\.object)
        }
        return object
    }

    private static func value(at path: ArraySlice<String>, in object: Object) -> (any Sendable)? {
        guard let field = path.first, let value = object[field] ?? nil else { return nil }
        if path.count == 1 {
            return value
        }
        guard let nested = value as? Object else { return nil }
        return Self.value(at: path.dropFirst(), in: nested)
    }

    private static func set(_ value: (any Sendable)?, at path: ArraySlice<String>, in object: inout Object) {
        guard let field = path.first else { return }
        if path.count == 1 {
            if let value {
                object[field] = value
            } else {
                object.removeValue(forKey: field)
            }
            return
        }
        var nested = object[field] as? Object ?? [:]
        set(value, at: path.dropFirst(), in: &nested)
        if nested.isEmpty {
            object.removeValue(forKey: field)
        } else {
            object[field] = nested
        }
    }
}

protocol WorkflowConditionEditingService: WorkflowTriggerEditingService {
    func loadWorkflowConditions(_ workflow: WorkflowConfigurationSummary) async throws -> WorkflowConditionTree
    func saveWorkflowConditions(_ workflow: WorkflowConfigurationSummary, conditions: WorkflowConditionTree)
        async throws -> WorkflowConfigurationSummary
}
