import FolioAPI
import Foundation
import Testing

struct WorkflowDefinitionWireTests {
    @Test func generatedClientDecodesAnOwnerWorkflowDefinitionOccurrence() throws {
        let source = """
            {
              "event": "workflow.disabled",
              "version": 1,
              "occurred_at": "2032-02-29T09:30:00Z",
              "workflow": {
                "id": "00000000-0000-0000-0000-000000000001",
                "vault_id": "00000000-0000-0000-0000-000000000002",
                "name": "File invoices",
                "enabled": false,
                "trigger": "document.uploaded",
                "actor_id": "owner-at-occurrence"
              }
            }
            """
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let delivery = try decoder.decode(Components.Schemas.WebhookPayload.self, from: Data(source.utf8))
        let workflow = try #require(delivery.workflow)
        #expect(delivery.event.rawValue == "workflow.disabled")
        #expect(workflow.enabled == false)
        #expect(workflow.actorId == "owner-at-occurrence")
    }
}
