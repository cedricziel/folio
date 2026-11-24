import Foundation
import Testing

@testable import FolioKit

@Suite(.serialized) struct WorkflowLifecycleServiceTests {
    private let vaultID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
    private let workflowID = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!

    @Test func trashListUsesExplicitVaultAndIncludesLifecycleAttribution() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows")
            #expect(request.url?.query == "deleted=true")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, "[\(workflowJSON(deleted: true))]")
        }
        let rows = try await service().workflowConfigurations(vaultID: vaultID, deleted: true)
        #expect(rows.count == 1)
        #expect(rows.first?.createdBy == "creator")
        #expect(rows.first?.updatedBy == "editor")
        #expect(rows.first?.stateTitle == "In Trash")
        #expect(rows.first?.deletedAt != nil)
        #expect(rows.first?.revision == 1)
    }

    @Test func retainedHistoryUsesSelectedVaultAndMapsFailedRunDetails() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())/runs")
            #expect(request.url?.query == "limit=50")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (
                200,
                """
                [{"id":"99999999-8888-4777-8666-555555555555","vault_id":"\(vaultID)",
                "workflow_id":"\(workflowID)","event":"document.uploaded","document_id":null,
                "status":"failed","attempts":3,"completed_actions":1,"last_error":"Receiver unavailable",
                "response_status":503,"created_at":"2026-11-02T12:34:56.123456789Z","finished_at":"2026-11-02T12:35:00Z"}]
                """
            )
        }
        let rows = try await service().workflowHistory(vaultID: vaultID, workflowID: workflowID)
        #expect(rows.first?.statusTitle == "Failed")
        #expect(rows.first?.attempts == 3)
        #expect(rows.first?.completedActions == 1)
        #expect(rows.first?.responseStatus == 503)
        #expect(rows.first?.lastError == "Receiver unavailable")
    }

    @Test func deletionUsesTheLifecycleEndpointWithoutARequestBody() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())")
            #expect(request.httpMethod == "DELETE")
            #expect(request.httpBody == nil)
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (204, "")
        }
        try await service().deleteWorkflow(vaultID: vaultID, workflowID: workflowID)
    }

    @Test func restorationReturnsTheActiveConfiguration() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())/restore")
            #expect(request.httpMethod == "POST")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, workflowJSON(deleted: false))
        }
        let restored = try await service().restoreWorkflow(vaultID: vaultID, workflowID: workflowID)
        #expect(restored.id == workflowID)
        #expect(restored.deletedAt == nil)
    }

    @Test func anotherVaultCannotSupplyConfigurationRows() async {
        LifecycleURLProtocol.handler = { _ in
            (200, "[\(workflowJSON(deleted: false))]")
        }
        await #expect(throws: FolioError.unreadableResponse) {
            try await service().workflowConfigurations(vaultID: UUID(), deleted: false)
        }
    }

    @Test func restoreRejectsAResponseThatIsStillInTrash() async {
        LifecycleURLProtocol.handler = { _ in (200, workflowJSON(deleted: true)) }
        await #expect(throws: FolioError.unreadableResponse) {
            try await service().restoreWorkflow(vaultID: vaultID, workflowID: workflowID)
        }
    }

    @Test func serverOwnerRestrictionsArePreserved() async {
        LifecycleURLProtocol.handler = { _ in (403, "{}") }
        await #expect(throws: FolioError.server(status: 403, message: nil)) {
            try await service().workflowConfigurations(vaultID: vaultID, deleted: false)
        }
    }

    @Test func generatedCatalogAndUpdateRoundTripEntryAndIngestionChoices() async throws {
        let entryIDs = [
            "document_note.created", "document_note.updated", "document_note.deleted",
            "document_comment.created", "document_comment.updated", "document_comment.deleted",
        ]
        let ingestionIDs = [
            "ingest.imported", "ingest.skipped", "ingest.duplicate", "ingest.failed",
            "source.failed", "source.recovered",
        ]
        let choiceIDs = entryIDs + ingestionIDs
        let field = WorkflowTriggerChoice.ConditionField(
            id: "event.subject_id", title: "Subject ID", valueType: "identifier")
        let definitions: [[String: Any]] = choiceIDs.map { id in
            [
                "event_type": id, "label": "Catalog label for \(id)",
                "subject_type": String(id.split(separator: ".")[0]),
                "target": entryIDs.contains(id) ? "document" : "event",
                "condition_fields": [["key": field.id, "label": field.title, "value_type": field.valueType]],
            ]
        }
        let catalog = try JSONSerialization.data(withJSONObject: definitions)
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/folio/api/workflows/triggers")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, String(decoding: catalog, as: UTF8.self))
        }
        let live = service()
        let choices = try await live.workflowTriggerChoices(vaultID: vaultID)
        #expect(choices.map(\.id) == choiceIDs)
        #expect(choices.map(\.target) == entryIDs.map { _ in .document } + ingestionIDs.map { _ in .event })
        #expect(choices.allSatisfy { $0.title == "Catalog label for \($0.id)" && $0.conditionFields == [field] })

        let path = "/folio/api/workflows/\(workflowID.uuidString.lowercased())"
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == path)
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, workflowJSON(deleted: false))
        }
        let summary = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        #expect(summary.revision == 1)

        for choice in choices {
            LifecycleURLProtocol.handler = { request in
                #expect(request.httpMethod == "PATCH")
                #expect(request.url?.path == path + "/trigger")
                #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
                guard let data = LifecycleURLProtocol.body(for: request),
                    let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
                else { return (400, "{}") }
                #expect(body["trigger"] as? String == choice.id)
                #expect(body["expected_revision"] as? Int64 == 1)
                #expect(Set(body.keys) == ["trigger", "expected_revision"])
                return (200, workflowJSON(deleted: false, trigger: choice.id, revision: 2))
            }
            let updated = try await live.changeWorkflowTrigger(summary, to: choice.id)
            #expect(updated.trigger == choice.id)
            #expect(updated.vaultID == vaultID)
            #expect(updated.revision == 2)
        }
    }

    @Test func aStaleTriggerEditCanReloadAndRetryTheNewRevision() async throws {
        let live = service()
        let path = "/folio/api/workflows/\(workflowID.uuidString.lowercased())"
        LifecycleURLProtocol.handler = { _ in (200, workflowJSON(deleted: false)) }
        let stale = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "PATCH")
            #expect(request.url?.path == path + "/trigger")
            return (409, #"{"error":"Workflow changed. Reload it before editing."}"#)
        }
        await #expect(throws: FolioError.server(status: 409, message: "Workflow changed. Reload it before editing.")) {
            try await live.changeWorkflowTrigger(stale, to: "document_note.created")
        }
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == path)
            return (200, workflowJSON(deleted: false, revision: 2, enabled: false))
        }
        let latest = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        #expect(!latest.enabled)
        #expect(latest.revision == 2)
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "PATCH")
            #expect(request.url?.path == path + "/trigger")
            guard let data = LifecycleURLProtocol.body(for: request),
                let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return (400, "{}") }
            #expect(body["expected_revision"] as? Int64 == 2)
            #expect(body["enabled"] == nil)
            return (200, workflowJSON(deleted: false, trigger: "document_note.created", revision: 3, enabled: false))
        }
        let saved = try await live.changeWorkflowTrigger(latest, to: "document_note.created")
        #expect(!saved.enabled)
        #expect(saved.revision == 3)
    }

    @Test func anOlderServerKeepsTheListReadableWithoutOfferingUnconditionalTriggerEdits() async throws {
        let live = service()
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            return (200, workflowJSON(deleted: false, revision: nil))
        }
        let workflow = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        #expect(workflow.revision == nil)
        await #expect(throws: FolioError.unreadableResponse) {
            try await live.changeWorkflowTrigger(workflow, to: "document_note.created")
        }
    }

    private func service() -> LiveFolioService {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [LifecycleURLProtocol.self]
        return LiveFolioService(
            baseURL: URL(string: "https://workflow-lifecycle.test/folio")!,
            session: URLSession(configuration: config), vaultID: UUID()
        )
    }

    private func workflowJSON(
        deleted: Bool, trigger: String = "document.uploaded", revision: Int64? = 1, enabled: Bool = true
    ) -> String {
        let revisionField = revision.map { "\"revision\":\($0)," } ?? ""
        return """
            {"id":"\(workflowID)","vault_id":"\(vaultID)","name":"File invoices","enabled":\(enabled),\(revisionField)
            "trigger":"\(trigger)","conditions":{"any":[{"event":{"actor":{"type":"system"}}}],"none":[]},
            "actions":[{"type":"webhook","url":"https://receiver.test/hook","method":"POST","secret":"********",
            "headers":{"Authorization":"********"}},{"type":"webhook","url":"https://receiver.test/hook","method":"POST",
            "headers":{"X-Custom-Credential":"********"}}],
            "created_at":"2026-11-02T12:34:56.123456789Z","updated_at":"2026-11-02T12:35:00Z",
            "deleted_at":\(deleted ? "\"2026-11-02T12:36:00Z\"" : "null"),"created_by":"creator","updated_by":"editor"}
            """
    }
}

private final class LifecycleURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: ((URLRequest) -> (Int, String))?

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "workflow-lifecycle.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        let (status, json) = Self.handler?(request) ?? (500, "{}")
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(json.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    static func body(for request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            guard count >= 0 else { return nil }
            if count == 0 { return data }
            data.append(buffer, count: count)
        }
    }

    override func stopLoading() {}
}
