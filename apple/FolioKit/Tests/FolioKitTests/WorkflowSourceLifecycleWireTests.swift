import FolioAPI
import Foundation
import Testing

@Test func managedSourceTriggerAndWebhookUseTheGeneratedContract() throws {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    for action in ["created", "updated", "deleted", "restored"] {
        let event = "source.\(action)"
        let payload = try decoder.decode(
            Components.Schemas.WebhookPayload.self,
            from: Data(
                """
                {"event":"\(event)","version":1,"occurred_at":"2026-11-23T14:00:00Z",
                "source":{"vault_id":"0e72ce5f-74dd-4471-8a87-beb383526a3a",
                "source_id":"3fa85f64-5717-4562-b3fc-2c963f66afa6","name":"Inbox source",
                "kind":"folder","enabled":false,"actor_id":null,"causation_id":null}}
                """.utf8)
        )
        let source = try #require(payload.source)
        #expect(payload.event.rawValue == event)
        #expect(source.sourceId == "3fa85f64-5717-4562-b3fc-2c963f66afa6")
        #expect(source.kind.rawValue == "folder")
        #expect(!source.enabled)
        #expect(source.actorId == nil)
    }
}
