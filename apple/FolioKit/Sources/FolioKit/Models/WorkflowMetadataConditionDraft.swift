import Foundation

struct WorkflowMetadataConditionDraft: Sendable {
    enum Key: String, Sendable {
        case before, after, state
        case addedID = "added_id"
        case removedID = "removed_id"
        case beforeID = "before_id"
        case afterID = "after_id"

        var isIdentifier: Bool {
            switch self {
            case .addedID, .removedID, .beforeID, .afterID: true
            default: false
            }
        }
    }

    enum Kind: String, Sendable {
        case title, tags, correspondent
        case documentType = "document_type"

        var keys: Set<Key> {
            switch self {
            case .title: [.before, .after]
            case .tags: [.addedID, .removedID, .state]
            case .correspondent, .documentType: [.beforeID, .afterID, .state]
            }
        }

        var states: [Option] {
            switch self {
            case .title: []
            case .tags: [.init("added", "Added only"), .init("removed", "Removed only"), .init("replaced", "Replaced")]
            case .correspondent, .documentType:
                [.init("set", "Assigned"), .init("replaced", "Replaced"), .init("cleared", "Cleared")]
            }
        }

        var taxonomyKind: TaxonomyKind? {
            switch self {
            case .title: nil
            case .tags: .tags
            case .correspondent: .correspondents
            case .documentType: .documentTypes
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

    struct Field: Identifiable, Sendable {
        let key: Key
        let title: String

        var id: Key {
            key
        }
    }

    let kind: Kind
    let fields: [Field]
    private var values: WorkflowConditionTree.Object

    init?(_ choice: WorkflowTriggerChoice, saved: WorkflowConditionTree.Object?) {
        let kind: Kind
        switch choice.id {
        case "document.title_changed": kind = .title
        case "document.tags_changed": kind = .tags
        case "document.correspondent_changed": kind = .correspondent
        case "document.document_type_changed": kind = .documentType
        default: return nil
        }
        let prefix = "event.document_change."
        var seen: Set<Key> = []
        let fields: [Field] = choice.conditionFields.compactMap { field in
            guard field.id.hasPrefix(prefix),
                let key = Key(rawValue: String(field.id.dropFirst(prefix.count))),
                kind.keys.contains(key), field.valueType == (key.isIdentifier ? "identifier" : "text"),
                seen.insert(key).inserted
            else { return nil }
            return Field(key: key, title: field.title)
        }
        guard !fields.isEmpty, saved == nil || (saved?["type"] as? String) == kind.rawValue else { return nil }
        if let saved {
            for field in fields {
                if let value = saved[field.key.rawValue] ?? nil, !(value is String) {
                    return nil
                }
            }
        }
        self.kind = kind
        self.fields = fields
        values = saved ?? [:]
    }

    var loadsIdentifiers: Bool {
        fields.contains { $0.key.isIdentifier }
    }

    var hasUnadvertisedValues: Bool {
        let editable = Set(fields.map { $0.key.rawValue })
        return values.keys.contains { $0 != "type" && !editable.contains($0) }
    }

    subscript(key: Key) -> String {
        get { values[key.rawValue] as? String ?? "" }
        set {
            if kind != .title && newValue.isEmpty {
                values.removeValue(forKey: key.rawValue)
            } else {
                values[key.rawValue] = newValue
            }
        }
    }

    func matchesTitle(_ key: Key) -> Bool {
        kind == .title && values[key.rawValue] is String
    }

    mutating func setTitleMatch(_ enabled: Bool, for key: Key) {
        if enabled {
            values[key.rawValue] = self[key]
        } else {
            values.removeValue(forKey: key.rawValue)
        }
    }

    var validationMessage: String? {
        if kind == .tags {
            if self[.state] == "added", !self[.removedID].isEmpty {
                return "Choose Replaced to match a removed tag when tags are also added."
            }
            if self[.state] == "removed", !self[.addedID].isEmpty {
                return "Choose Replaced to match an added tag when tags are also removed."
            }
            if !self[.addedID].isEmpty, self[.addedID] == self[.removedID] {
                return "Choose different added and removed tags."
            }
        }
        if kind == .correspondent || kind == .documentType {
            if self[.state] == "set", !self[.beforeID].isEmpty {
                return "An assignment has no previous value."
            }
            if self[.state] == "cleared", !self[.afterID].isEmpty {
                return "Clearing the value leaves the new value empty."
            }
            if !self[.beforeID].isEmpty, self[.beforeID] == self[.afterID] {
                return "Choose different previous and new values."
            }
        }
        return nil
    }

    var wireValue: WorkflowConditionTree.Object? {
        var object = values
        object.removeValue(forKey: "type")
        guard !object.isEmpty else { return nil }
        object["type"] = kind.rawValue
        return object
    }
}
