import FolioAPI
import Foundation
import Testing

struct WorkflowRunWireTests {
    @Test func generatedInputRoundTripsTheSourceWorkflowCondition() throws {
        let sourceID = "00000000-0000-0000-0000-000000000001"
        let source = """
            {
              "name": "On failure",
              "trigger": "workflow_run.failed",
              "conditions": {"all": [{"event": {"workflow_id": "\(sourceID)"}}]},
              "actions": [{"type": "webhook", "url": "https://example.test/notify"}]
            }
            """
        let input = try JSONDecoder().decode(Components.Schemas.WorkflowInput.self, from: Data(source.utf8))
        let output = try JSONSerialization.jsonObject(with: JSONEncoder().encode(input))
        let fields = try #require(output as? [String: Any])
        let conditions = try #require(fields["conditions"] as? [String: Any])
        let all = try #require(conditions["all"] as? [[String: Any]])
        let event = try #require(all.first?["event"] as? [String: String])
        #expect(fields["trigger"] as? String == "workflow_run.failed")
        #expect(event["workflow_id"] == sourceID)
    }

    @Test func generatedDeliveryDecodesTheWorkflowRunOutcome() throws {
        let source = """
            {
              "event": "workflow_run.succeeded",
              "version": 1,
              "occurred_at": "2032-02-29T09:30:00Z",
              "workflow_run": {
                "id": "00000000-0000-0000-0000-000000000001",
                "workflow_id": "00000000-0000-0000-0000-000000000002",
                "vault_id": "00000000-0000-0000-0000-000000000003"
              }
            }
            """
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let delivery = try decoder.decode(Components.Schemas.WebhookPayload.self, from: Data(source.utf8))
        #expect(delivery.event.rawValue == "workflow_run.succeeded")
        #expect(delivery.workflowRun?.workflowId == "00000000-0000-0000-0000-000000000002")
    }
}
