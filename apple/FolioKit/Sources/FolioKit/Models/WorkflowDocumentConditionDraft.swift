import Foundation
import OpenAPIRuntime

struct WorkflowDocumentConditionDraft: Equatable, Sendable {
    enum Key: String, Sendable {
        case before, after, state, origin, reason, number, source

        var valueType: String {
            switch self {
            case .before, .after: "date"
            case .number: "integer"
            default: "text"
            }
        }

        var options: [Option] {
            switch self {
            case .state:
                [
                    .init("set", "Date set"), .init("replaced", "Date changed"), .init("cleared", "Date cleared"),
                ]
            case .origin:
                [
                    .init("human", "Person"), .init("system", "Folio"),
                    .init("legacy_classification", "Earlier classification"),
                ]
            case .reason:
                [
                    .init("user", "Marked by a person"), .init("system", "Marked by Folio"),
                    .init("file_replaced", "File replaced"), .init("file_restored", "Earlier revision restored"),
                ]
            case .source:
                [
                    .init("upload", "First upload"), .init("replacement", "Replacement file"),
                    .init("restoration", "Restored revision"),
                ]
            default: []
            }
        }
    }

    struct Option: Identifiable, Sendable {
        let id: String
        let title: String

        init(_ id: String, _ title: String) {
            self.id = id
            self.title = title
        }
    }

    struct Field: Identifiable, Equatable, Sendable {
        var id: Key { key }
        let key: Key
        let title: String
    }

    let triggerID: String
    let type: String
    let fields: [Field]
    private var values: [Key: String] = [:]
    private var original: OpenAPIObjectContainer?
    private static let prefix = "event.document_change."

    init?(_ choice: WorkflowTriggerChoice) {
        let allowed: Set<Key>
        switch choice.id {
        case "document.date_changed":
            type = "date"
            allowed = [.before, .after, .state]
        case "document.reviewed":
            type = "review"
            allowed = [.origin]
        case "document.unreviewed":
            type = "review"
            allowed = [.origin, .reason]
        case "document.revision_created":
            type = "revision"
            allowed = [.number, .source]
        case "document.revision_restored":
            type = "revision"
            allowed = [.number]
        default:
            return nil
        }
        triggerID = choice.id
        var seen: Set<Key> = []
        fields = choice.conditionFields.compactMap { field in
            guard field.id.hasPrefix(Self.prefix),
                let key = Key(rawValue: String(field.id.dropFirst(Self.prefix.count))),
                allowed.contains(key), key.valueType == field.valueType, seen.insert(key).inserted
            else { return nil }
            return Field(key: key, title: field.title)
        }
        guard !fields.isEmpty else { return nil }
    }

    subscript(key: Key) -> String {
        get { values[key] ?? "" }
        set { values[key] = newValue.isEmpty ? nil : newValue }
    }

    var validationMessage: String? {
        if fields.contains(where: { field in
            field.key.valueType == "date" && !self[field.key].isEmpty
                && DocumentDate(rawValue: self[field.key]) == nil
        }) {
            return "Enter a valid date as YYYY-MM-DD."
        }
        if self[.state] == "set", !self[.before].isEmpty {
            return "A date set for the first time has no previous date."
        }
        if self[.state] == "cleared", !self[.after].isEmpty {
            return "Clearing a date leaves the new date empty."
        }
        if !self[.number].isEmpty {
            let minimum = triggerID == "document.revision_restored" ? 2 : 1
            if let number = Int(self[.number]), number >= minimum {
                return nil
            }
            return "Enter a whole revision number of at least \(minimum)."
        }
        return nil
    }

    mutating func load(_ object: [String: (any Sendable)?]) throws {
        guard object["type"] as? String == type else { throw FolioError.unreadableResponse }
        original = try OpenAPIObjectContainer(unvalidatedValue: object)
        for field in fields {
            let key = field.key
            guard let value = object[key.rawValue] ?? nil else { continue }
            if key == .number, let number = value as? Int {
                self[key] = String(number)
            } else if key != .number, let text = value as? String {
                self[key] = text
            } else {
                throw FolioError.unreadableResponse
            }
        }
    }

    func shouldSubmit(startingTrigger: String) -> Bool {
        triggerID == startingTrigger || wireValue != nil
    }

    var wireValue: [String: (any Sendable)?]? {
        guard validationMessage == nil else { return nil }
        var object = original?.value ?? [:]
        object.removeValue(forKey: "type")
        for field in fields {
            let key = field.key
            let value = self[key]
            if value.isEmpty {
                object.removeValue(forKey: key.rawValue)
            } else if key == .number {
                object[key.rawValue] = Int(value)
            } else {
                object[key.rawValue] = value
            }
        }
        guard !object.isEmpty else { return nil }
        object["type"] = type
        return object
    }
}

protocol WorkflowDocumentConditionEditingService: WorkflowTriggerEditingService {
    func loadWorkflowDocumentConditions(
        _ workflow: WorkflowConfigurationSummary, choice: WorkflowTriggerChoice
    ) async throws -> WorkflowDocumentConditionDraft?

    func saveWorkflowTrigger(
        _ workflow: WorkflowConfigurationSummary, choice: WorkflowTriggerChoice,
        documentConditions: WorkflowDocumentConditionDraft?
    ) async throws -> WorkflowConfigurationSummary
}
