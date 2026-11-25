import FolioKit
import Foundation
import SwiftUI
import Synchronization

struct WorkflowLifecycleFixture: View {
    @State private var service = WorkflowLifecycleFixtureService()

    @State private var editorService = WorkflowEditorFixtureProtocol.service()
    @State private var memberService = WorkflowMemberFixtureProtocol.service()

    var body: some View {
        let readOnly = ProcessInfo.processInfo.arguments.contains("--workflow-member")
        let workflowService: any WorkflowLifecycleService =
            if readOnly { memberService } else if usesGeneratedEditing { editorService } else { service }
        WorkflowLifecycleView(
            vaultID: WorkflowLifecycleFixtureService.vaultID,
            service: workflowService,
            canManage: !ProcessInfo.processInfo.arguments.contains("--read-only")
        )
    }

    private var usesGeneratedEditing: Bool {
        let arguments = ProcessInfo.processInfo.arguments
        return arguments.contains("--workflow-action-order") || arguments.contains("--workflow-create")
            || arguments.contains("--workflow-schedule") || arguments.contains("--workflow-document-actions")
            || arguments.contains("--workflow-create-multiple")
            || arguments.contains("--workflow-custom-field-actions")
    }
}

@MainActor final class WorkflowLifecycleFixtureService: WorkflowLifecycleService, WorkflowTriggerEditingService {
    nonisolated static let vaultID = UUID(uuidString: "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff")!
    nonisolated static let workflowID = UUID(uuidString: "00000000-0000-4000-8000-000000000001")!
    private let date = Date(timeIntervalSince1970: 1_793_620_800)
    private var deleted = false
    private var triggerID = "document.uploaded"
    private var revision: Int64 = 1
    private var enabled = true
    private var concurrentEdit = ProcessInfo.processInfo.arguments.contains("--concurrent-workflow-edit")
    private var failList = ProcessInfo.processInfo.arguments.contains("--initial-load-error")
    private var failHistory = ProcessInfo.processInfo.arguments.contains("--workflow-history-error")
    private var failDelete = ProcessInfo.processInfo.arguments.contains("--workflow-delete-error")

    func workflowConfigurations(vaultID _: UUID, deleted: Bool) async throws -> [WorkflowConfigurationSummary] {
        if failList {
            failList = false
            throw FolioError.unreachable
        }
        return self.deleted == deleted ? [configuration] : []
    }

    func workflowHistory(vaultID _: UUID, workflowID _: UUID) async throws -> [WorkflowExecutionSummary] {
        if failHistory {
            failHistory = false
            throw FolioError.unreachable
        }
        return [
            WorkflowExecutionSummary(
                id: UUID(uuidString: "00000000-0000-4000-8000-000000000002")!,
                vaultID: Self.vaultID, workflowID: Self.workflowID, event: "document.uploaded",
                status: "failed", attempts: 3, completedActions: 1,
                lastError: "Receiver unavailable", responseStatus: 503, createdAt: date, finishedAt: date
            )
        ]
    }

    func deleteWorkflow(vaultID _: UUID, workflowID _: UUID) async throws {
        if failDelete {
            failDelete = false
            throw FolioError.unreachable
        }
        deleted = true
    }

    func restoreWorkflow(vaultID _: UUID, workflowID _: UUID) async throws -> WorkflowConfigurationSummary {
        deleted = false
        return configuration
    }

    func workflowTriggerChoices(vaultID _: UUID) async throws -> [WorkflowTriggerChoice] {
        [
            .init(id: "document.uploaded", title: "When a document is added", target: .document, conditionFields: []),
            .init(id: "document_note.created", title: "When a note is created", target: .document, conditionFields: []),
            .init(
                id: "document_comment.created", title: "When a comment is created", target: .document, conditionFields: []
            ),
            .init(id: "source.failed", title: "When a source becomes unavailable", target: .event, conditionFields: []),
        ]
    }

    func workflowConfiguration(vaultID: UUID, workflowID: UUID) async throws -> WorkflowConfigurationSummary {
        guard vaultID == Self.vaultID, workflowID == Self.workflowID, !deleted else {
            throw FolioError.unreadableResponse
        }
        return configuration
    }

    func changeWorkflowTrigger(_ workflow: WorkflowConfigurationSummary, to triggerID: String) async throws
        -> WorkflowConfigurationSummary
    {
        guard workflow.vaultID == Self.vaultID, workflow.id == Self.workflowID, !deleted else {
            throw FolioError.unreadableResponse
        }
        if concurrentEdit {
            concurrentEdit = false
            enabled = false
            revision += 1
        }
        guard workflow.revision == revision else { throw FolioError.server(status: 409, message: nil) }
        self.triggerID = triggerID
        revision += 1
        return configuration
    }

    private var configuration: WorkflowConfigurationSummary {
        WorkflowConfigurationSummary(
            id: Self.workflowID, vaultID: Self.vaultID, name: "Invoice notifications", enabled: enabled,
            trigger: triggerID, createdAt: date, updatedAt: date, deletedAt: deleted ? date : nil,
            createdBy: "Taylor", updatedBy: "Morgan", revision: revision
        )
    }
}

private final class WorkflowEditorFixtureProtocol: URLProtocol, @unchecked Sendable {
    private struct SavedWorkflow {
        var revision = 7
        var name = "Invoice notifications"
        var enabled = true
        var trigger = "document.uploaded"
        var triggers: [String]?
        var cadence = Data("null".utf8)
        var conditions = Data(#"{"any":[]}"#.utf8)
        var actions = Data(
            """
            [{"type":"webhook","url":"https://first.test/hook","method":"POST",
              "headers":{"Authorization":"********"}},
             {"type":"webhook","url":"https://second.test/hook","method":"POST",
              "secret":"********","headers":{}}]
            """.utf8)
    }

    private static let saved = Mutex(initialWorkflow())
    private static let creatingMultiple = ProcessInfo.processInfo.arguments.contains("--workflow-create-multiple")
    private static let creatingWorkflow =
        creatingMultiple || ProcessInfo.processInfo.arguments.contains("--workflow-create")
    private static let createdSelection = ["document_note.created", "document.uploaded"]
    private static let documentActions = ProcessInfo.processInfo.arguments.contains("--workflow-document-actions")
    private static let scheduleEditing = ProcessInfo.processInfo.arguments.contains("--workflow-schedule")

    private static func initialWorkflow() -> SavedWorkflow {
        var workflow = SavedWorkflow()
        if documentActions || ProcessInfo.processInfo.arguments.contains("--workflow-custom-field-actions") {
            workflow.actions = Data(#"[{"type":"set_reviewed","reviewed":false}]"#.utf8)
            workflow.conditions = Data("{}".utf8)
        }
        if scheduleEditing {
            workflow.trigger = "workflow.scheduled"
            workflow.cadence = Data(
                #"{"type":"recurring","first_at":"2032-02-29T09:30:00.123456789Z","interval_seconds":3600}"#.utf8)
            workflow.conditions = Data("{}".utf8)
        }
        return workflow
    }
    private static let id = WorkflowLifecycleFixtureService.workflowID.uuidString.lowercased()
    private static let vault = WorkflowLifecycleFixtureService.vaultID.uuidString.lowercased()
    private static let host = "workflow-editor-fixture.test"
    private static let customField = "00000000-0000-4000-8000-000000000031"

    static func service() -> LiveFolioService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [Self.self]
        return LiveFolioService(
            baseURL: URL(string: "https://\(host)")!,
            session: URLSession(configuration: configuration), vaultID: WorkflowLifecycleFixtureService.vaultID
        )
    }

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == host
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let (status, json) = Self.response(for: request)
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(json.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func response(for request: URLRequest) -> (Int, String) {
        guard request.value(forHTTPHeaderField: "X-Folio-Vault") == vault else { return (403, "{}") }
        switch (request.httpMethod, request.url?.path) {
        case ("GET", "/api/vaults/\(vault)/metadata-fields"):
            return (
                200,
                """
                [{"id":"\(customField)","vault_id":"\(vault)","name":"Invoice total","kind":"money",
                "required":false,"unique":false,"created_at":"2026-11-02T12:34:56Z",
                "updated_at":"2026-11-02T12:35:00Z"}]
                """
            )
        case ("GET", "/api/workflows"):
            let awaitsCreation = creatingWorkflow && saved.withLock { $0.revision == 7 }
            return (200, awaitsCreation ? "[]" : "[\(workflow)]")
        case ("POST", "/api/workflows"):
            guard creatingWorkflow,
                let data = body(for: request),
                let input = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
                let name = input["name"] as? String, name == "Notify intake",
                input["enabled"] as? Bool == false,
                let actions = input["actions"] as? [[String: Any]],
                actions.count == 1, actions.first?["url"] as? String == "https://new.test/hook",
                let encoded = try? JSONSerialization.data(withJSONObject: actions)
            else { return (400, #"{"error":"Unexpected workflow creation input"}"#) }
            let legacy = input["trigger"] as? String
            let ordered = input["triggers"] as? [String]
            let validSelection =
                creatingMultiple
                ? legacy == nil && ordered == createdSelection : legacy == "document.uploaded" && ordered == nil
            guard validSelection else { return (400, #"{"error":"Unexpected trigger selection"}"#) }
            saved.withLock {
                $0.revision += 1
                $0.name = name
                $0.trigger = ordered?.first ?? "document.uploaded"
                $0.triggers = ordered
                $0.enabled = false
                $0.actions = encoded
                $0.conditions = Data("{}".utf8)
            }
            return (201, workflow)
        case ("GET", "/api/workflows/triggers"):
            return (
                200,
                """
                [{"event_type":"document.uploaded","label":"When a document is added","subject_type":"document",
                "target":"document","condition_fields":[{"key":"filename_glob","label":"Filename at upload","value_type":"text"}]},
                {"event_type":"document_note.created","label":"When a note is created","subject_type":"document_note",
                "target":"document","condition_fields":[]},
                {"event_type":"workflow.scheduled","label":"On a schedule","subject_type":"workflow",
                "target":"event","condition_fields":[]}]
                """
            )
        case ("GET", "/api/workflows/\(id)/runs"):
            return (200, "[]")
        case ("GET", "/api/workflows/\(id)"):
            return (200, workflow)
        case ("PUT", "/api/workflows/\(id)/definition"):
            guard let data = body(for: request),
                let input = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
                let name = input["name"] as? String,
                let enabled = input["enabled"] as? Bool,
                let trigger = input["trigger"] as? String,
                let cadence = try? JSONSerialization.data(
                    withJSONObject: input["cadence"] ?? NSNull(), options: .fragmentsAllowed),
                let filters = input["conditions"] as? [String: Any],
                let conditions = try? JSONSerialization.data(withJSONObject: filters),
                let actions = input["actions"] as? [[String: Any]],
                let encodedActions = try? JSONSerialization.data(withJSONObject: actions)
            else { return (400, "{}") }
            let sources = request.value(forHTTPHeaderField: "X-Folio-Workflow-Action-Sources")
            if ProcessInfo.processInfo.arguments.contains("--workflow-action-order") {
                guard sources == "1,0", actions.first?["url"] as? String == "https://second.test/hook",
                    actions.first?["secret"] as? String == "********",
                    (actions.last?["headers"] as? [String: String])?["Authorization"] == "********"
                else { return (400, #"{"error":"Unexpected action provenance"}"#) }
            }
            if documentActions {
                guard sources == "-,0", actions.count == 2,
                    actions.first?["type"] as? String == "set_document_date",
                    actions.first?["date"] as? String == "2028-02-29",
                    actions.last?["type"] as? String == "set_reviewed",
                    actions.last?["reviewed"] as? Bool == false
                else { return (400, #"{"error":"Unexpected document action order"}"#) }
            }
            if ProcessInfo.processInfo.arguments.contains("--workflow-custom-field-actions") {
                guard sources == "-,0", actions.count == 2,
                    actions[0]["type"] as? String == "set_metadata",
                    actions[0]["field_id"] as? String == customField,
                    let value = actions[0]["value"] as? [String: Any],
                    value["kind"] as? String == "money",
                    let money = value["value"] as? [String: String],
                    money["amount"] == "9007199254740993.010",
                    money["currency"] == "EUR",
                    actions[1]["type"] as? String == "set_reviewed"
                else { return (400, #"{"error":"Unexpected custom-field actions"}"#) }
            }
            if scheduleEditing {
                guard trigger == "workflow.scheduled", let schedule = input["cadence"] as? [String: String],
                    schedule == ["type": "once", "at": "2032-02-29T09:30:00.123456789Z"]
                else { return (400, #"{"error":"Unexpected cadence"}"#) }
            }
            let triggers = input["triggers"] as? [String]
            if creatingMultiple {
                guard trigger == createdSelection[0], triggers == createdSelection
                else { return (400, #"{"error":"Workflow settings changed the trigger order"}"#) }
            }
            let query = request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
            let expected = query?.queryItems?.first(where: { $0.name == "expected_revision" })?.value
            let updated = saved.withLock { saved in
                guard expected == String(saved.revision) else { return false }
                saved.revision += 1
                saved.name = name
                saved.enabled = enabled
                saved.trigger = trigger
                saved.triggers = triggers
                saved.cadence = cadence
                saved.conditions = conditions
                saved.actions = encodedActions
                return true
            }
            return updated ? (200, workflow) : (409, #"{"error":"Revision changed"}"#)
        default:
            return (404, "{}")
        }
    }

    private static var workflow: String {
        let snapshot = saved.withLock { $0 }
        let filters = String(decoding: snapshot.conditions, as: UTF8.self)
        let actions = String(decoding: snapshot.actions, as: UTF8.self)
        let cadence = String(decoding: snapshot.cadence, as: UTF8.self)
        let triggers =
            snapshot.triggers.flatMap { try? JSONEncoder().encode($0) }
            .map { "\"triggers\":" + String(decoding: $0, as: UTF8.self) + "," } ?? ""
        let name = (try? JSONEncoder().encode(snapshot.name)).map { String(decoding: $0, as: UTF8.self) } ?? "null"
        return """
            {"id":"\(id)","vault_id":"\(vault)","name":\(name),"enabled":\(snapshot.enabled),
            "trigger":"\(snapshot.trigger)",\(triggers)"cadence":\(cadence),"conditions":\(filters),
            "actions":\(actions),"revision":\(snapshot.revision),
            "created_at":"2026-11-02T12:34:56Z","updated_at":"2026-11-02T12:35:00Z","deleted_at":null}
            """
    }

    private static func body(for request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count == 0 { return data }
            guard count > 0 else { return nil }
            data.append(buffer, count: count)
        }
    }
}

private final class WorkflowMemberFixtureProtocol: URLProtocol, @unchecked Sendable {
    private static let host = "workflow-member-fixture.test"
    private static let id = "00000000-0000-4000-8000-000000000001"
    private static let vault = WorkflowLifecycleFixtureService.vaultID.uuidString.lowercased()

    static func service() -> LiveFolioService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [Self.self]
        return LiveFolioService(
            baseURL: URL(string: "https://\(host)")!, session: URLSession(configuration: configuration)
        )
    }

    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == host }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let (status, json) = Self.response(for: request)
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(json.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func response(for request: URLRequest) -> (Int, String) {
        guard request.httpMethod == "GET", request.value(forHTTPHeaderField: "X-Folio-Vault") == vault else {
            return (403, "{}")
        }
        switch request.url?.path {
        case "/api/workflows/summaries":
            return (
                200,
                """
                [{"id":"\(id)","vault_id":"\(vault)","name":"Invoice notifications","enabled":true,
                  "trigger":"document.processed","action_count":2,"trigger_selection":[
                  {"event_type":"document.processed","label":"When a document has been read"},
                  {"event_type":"saved_view.updated","label":"When a shared saved view is updated"}],
                  "created_at":"2026-11-02T12:34:56Z","updated_at":"2026-11-02T12:35:00Z"}]
                """
            )
        case "/api/workflows/\(id)/run-summaries":
            return (
                200,
                """
                [{"id":"00000000-0000-4000-8000-000000000002","vault_id":"\(vault)","workflow_id":"\(id)",
                  "event":"document.uploaded","status":"failed","attempts":3,"completed_actions":1,
                  "created_at":"2026-11-02T12:34:56Z","finished_at":"2026-11-02T12:35:00Z"}]
                """
            )
        default:
            return (403, "{}")
        }
    }
}
