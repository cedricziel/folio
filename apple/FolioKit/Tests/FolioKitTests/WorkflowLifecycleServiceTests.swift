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

    private func service() -> LiveFolioService {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [LifecycleURLProtocol.self]
        return LiveFolioService(
            baseURL: URL(string: "https://workflow-lifecycle.test/folio")!,
            session: URLSession(configuration: config), vaultID: UUID()
        )
    }

    private func workflowJSON(deleted: Bool) -> String {
        """
        {"id":"\(workflowID)","vault_id":"\(vaultID)","name":"File invoices","enabled":true,
        "trigger":"document.uploaded","conditions":{},"actions":[{"type":"webhook","secret":"********"}],
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

    override func stopLoading() {}
}
