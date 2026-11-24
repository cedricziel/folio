import FolioAPI
import Foundation

struct WorkflowActionDraft: Sendable {
    typealias Action = Components.Schemas.WorkflowAction

    enum Kind: String, Identifiable, Sendable {
        case webhook, addTags, correspondent, documentType, skipImport
        case rename, removeTags, clearCorrespondent, clearDocumentType, trash
        case documentDate, clearDocumentDate, reviewed

        var id: Self { self }

        var title: String {
            switch self {
            case .webhook: "Webhook"
            case .addTags: "Add tags"
            case .correspondent: "Set correspondent"
            case .documentType: "Set document type"
            case .skipImport: "Skip import"
            case .rename: "Rename document"
            case .removeTags: "Remove tags"
            case .clearCorrespondent: "Clear correspondent"
            case .clearDocumentType: "Clear document type"
            case .trash: "Move document to Trash"
            case .documentDate: "Set document date"
            case .clearDocumentDate: "Clear document date"
            case .reviewed: "Set review state"
            }
        }

        static func available(for target: WorkflowTriggerChoice.Target) -> [Self] {
            switch target {
            case .document:
                [
                    .webhook, .rename, .addTags, .removeTags, .correspondent, .clearCorrespondent, .documentType,
                    .clearDocumentType, .documentDate, .clearDocumentDate, .reviewed, .trash,
                ]
            case .event: [.webhook]
            case .ingestCandidate: [.skipImport]
            }
        }
    }

    struct Row: Identifiable, Sendable {
        private static let mask = "********"

        let id = UUID()
        let kind: Kind
        let sourceIndex: Int?
        private let savedURL: String?
        var url = ""
        var signingSecret = ""
        var headerLines = ""
        var references = ""
        var title = ""
        var date = ""
        var reviewed = false

        init(kind: Kind) {
            self.kind = kind
            sourceIndex = nil
            savedURL = nil
        }

        init(saved: Action, at index: Int) {
            sourceIndex = index
            if case .case1(let webhook) = saved {
                savedURL = webhook.url
            } else {
                savedURL = nil
            }
            switch saved {
            case .case1(let webhook):
                kind = .webhook
                url = webhook.url
                signingSecret = Self.masked(webhook.secret ?? "")
                let headers = webhook.headers?.additionalProperties ?? [:]
                headerLines = headers.sorted(by: { $0.key < $1.key }).map {
                    "\($0.key): \(Self.masked($0.value))"
                }.joined(separator: "\n")
            case .case2(let tags):
                kind = .addTags
                references = tags.tagIds.joined(separator: ", ")
            case .case3(let correspondent):
                kind = .correspondent
                references = correspondent.correspondentId
            case .case4(let documentType):
                kind = .documentType
                references = documentType.documentTypeId
            case .case5:
                kind = .skipImport
            case .case6(let rename):
                kind = .rename
                title = rename.title
            case .case7(let tags):
                kind = .removeTags
                references = tags.tagIds.joined(separator: ", ")
            case .case8:
                kind = .clearCorrespondent
            case .case9:
                kind = .clearDocumentType
            case .case10:
                kind = .trash
            case .case11(let setDate):
                kind = .documentDate
                date = setDate.date
            case .case12:
                kind = .clearDocumentDate
            case .case13(let setReviewed):
                kind = .reviewed
                reviewed = setReviewed.reviewed
            }
        }

        var destination: String? {
            let displayURL = url.trimmingCharacters(in: .whitespacesAndNewlines)
            guard kind == .webhook, var address = URLComponents(string: displayURL), address.host != nil else {
                return nil
            }
            address.user = nil
            address.password = nil
            address.query = nil
            address.fragment = nil
            return address.string
        }

        func prepare() throws -> Action {
            switch kind {
            case .webhook:
                try webhook()
            case .addTags:
                try .case2(.init(tagIds: tagIDs(), _type: .addTags))
            case .correspondent:
                try .case3(
                    .init(correspondentId: identifier(references, field: "correspondent"), _type: .setCorrespondent))
            case .documentType:
                try .case4(
                    .init(documentTypeId: identifier(references, field: "document type"), _type: .setDocumentType))
            case .skipImport:
                .case5(.init(_type: .skipImport))
            case .rename:
                try .case6(.init(title: documentTitle(), _type: .renameDocument))
            case .removeTags:
                try .case7(.init(tagIds: tagIDs(), _type: .removeTags))
            case .clearCorrespondent:
                .case8(.init(_type: .clearCorrespondent))
            case .clearDocumentType:
                .case9(.init(_type: .clearDocumentType))
            case .trash:
                .case10(.init(_type: .trashDocument))
            case .documentDate:
                try .case11(.init(date: documentDate(), _type: .setDocumentDate))
            case .clearDocumentDate:
                .case12(.init(_type: .clearDocumentDate))
            case .reviewed:
                .case13(.init(reviewed: reviewed, _type: .setReviewed))
            }
        }

        private func documentTitle() throws -> String {
            guard !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw invalid("Enter a document title.")
            }
            guard !title.contains("\0") else { throw invalid("Document title cannot contain a NUL character.") }
            return title
        }

        private func documentDate() throws -> String {
            let value = date.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let day = DocumentDate(rawValue: value) else { throw invalid("Enter a valid YYYY-MM-DD date.") }
            return day.rawValue
        }

        private static func masked(_ value: String) -> String {
            value.isEmpty ? "" : mask
        }

        private func webhook() throws -> Action {
            let trimmed = url.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let address = URLComponents(string: trimmed),
                let scheme = address.scheme?.lowercased(), ["http", "https"].contains(scheme),
                let host = address.host, !host.isEmpty
            else { throw invalid("Enter an HTTP or HTTPS webhook URL.") }

            let destination = url == savedURL ? url : trimmed
            let headers = try parseHeaders()
            if signingSecret == Self.mask || headers.values.contains(Self.mask) {
                guard savedURL == destination else {
                    throw invalid("Replace or clear the saved credentials when the webhook URL changes.")
                }
            }
            return .case1(
                .init(
                    headers: headers.isEmpty ? nil : .init(additionalProperties: headers),
                    method: .post, secret: signingSecret.isEmpty ? nil : signingSecret, _type: .webhook,
                    url: destination
                ))
        }

        private func parseHeaders() throws -> [String: String] {
            var values: [String: String] = [:]
            for raw in headerLines.split(whereSeparator: \.isNewline) {
                let line = raw.trimmingCharacters(in: .whitespaces)
                guard !line.isEmpty else { continue }
                let pair = line.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
                guard pair.count == 2 else { throw invalid("Use Name: Value for each webhook header.") }
                let name = pair[0].trimmingCharacters(in: .whitespaces)
                guard !name.isEmpty else { throw invalid("Enter a name for each webhook header.") }
                guard values[name] == nil else { throw invalid("Enter each webhook header once.") }
                values[name] = pair[1].trimmingCharacters(in: .whitespaces)
            }
            return values
        }

        private func tagIDs() throws -> [String] {
            let tags = references.split(separator: ",", omittingEmptySubsequences: false)
            guard !tags.isEmpty else { throw invalid("Enter at least one tag ID.") }
            return try tags.map { try identifier(String($0), field: "tag") }
        }

        private func identifier(_ value: String, field: String) throws -> String {
            guard let id = UUID(uuidString: value.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                throw invalid("Enter a valid \(field) ID.")
            }
            return id.uuidString.lowercased()
        }
    }

    let revision: Int64?
    var rows: [Row]

    init(revision: Int64? = nil, actions: [Action] = []) {
        self.revision = revision
        rows = actions.enumerated().map { Row(saved: $0.element, at: $0.offset) }
    }

    mutating func move(_ id: Row.ID, by offset: Int) {
        guard let from = rows.firstIndex(where: { $0.id == id }),
            rows.indices.contains(from + offset)
        else { return }
        rows.swapAt(from, from + offset)
    }

    func prepare(for target: WorkflowTriggerChoice.Target) throws -> (actions: [Action], sources: [String]) {
        guard !rows.isEmpty else { throw invalid("Add at least one action.") }
        if let trash = rows.firstIndex(where: { $0.kind == .trash }), trash != rows.count - 1 {
            throw invalid("Move to Trash must be the final action.")
        }
        let available = Kind.available(for: target)
        let actions = try rows.map { row in
            guard available.contains(row.kind) else {
                throw invalid("\(row.kind.title) is unavailable for this trigger.")
            }
            return try row.prepare()
        }
        return (actions, rows.map { $0.sourceIndex.map(String.init) ?? "-" })
    }

    func validationMessage(for target: WorkflowTriggerChoice.Target) -> String? {
        do {
            _ = try prepare(for: target)
            return nil
        } catch FolioError.server(_, let message) {
            return message
        } catch {
            return "Check the workflow actions."
        }
    }
}

private func invalid(_ message: String) -> FolioError {
    .server(status: 400, message: message)
}

protocol WorkflowActionEditingService: WorkflowConditionEditingService {
    func loadWorkflowActions(_ workflow: WorkflowConfigurationSummary) async throws -> WorkflowActionDraft
    func saveWorkflowActions(
        _ workflow: WorkflowConfigurationSummary, actions: WorkflowActionDraft, choice: WorkflowTriggerChoice
    ) async throws -> WorkflowConfigurationSummary
}

protocol WorkflowActionTaxonomyService: Sendable {
    func workflowActionTaxonomy(_ kind: TaxonomyKind, vaultID: UUID) async throws -> [TaxonomyEntry]
}
