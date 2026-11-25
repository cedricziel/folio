import FolioAPI
import Foundation
import Testing

@Test func classificationTriggerValuesDecodeFromTheCatalog() throws {
    for subject in ["tag", "correspondent", "document_type"] {
        for action in ["created", "updated", "deleted", "restored"] {
            let event = "\(subject).\(action)"
            let choice = try JSONDecoder().decode(
                Components.Schemas.WorkflowTriggerDefinition.self,
                from: Data(
                    """
                    {"event_type":"\(event)","label":"Classification change","subject_type":"\(subject)",
                    "target":"event","condition_fields":[{"key":"event.subject_id",
                    "label":"Subject ID","value_type":"identifier"}]}
                    """.utf8)
            )
            #expect(Components.Schemas.WorkflowTrigger(rawValue: choice.eventType)?.rawValue == event)
            #expect(choice.target.rawValue == "event")
        }
    }
}

@Test func classificationWebhookDecodesTheCapturedRow() throws {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    let payload = try decoder.decode(
        Components.Schemas.WebhookPayload.self,
        from: Data(
            """
            {"event":"tag.updated","version":1,"occurred_at":"2026-11-23T14:00:00Z",
            "classification":{"vault_id":"0e72ce5f-74dd-4471-8a87-beb383526a3a",
            "subject_id":"3fa85f64-5717-4562-b3fc-2c963f66afa6","name":"Invoices","color":"#123456",
            "actor_id":null,"causation_id":"24201525-7afd-4684-baea-eb1ed333c209"}}
            """.utf8)
    )
    let classification = try #require(payload.classification)
    #expect(payload.event.rawValue == "tag.updated")
    #expect(classification.subjectId == "3fa85f64-5717-4562-b3fc-2c963f66afa6")
    #expect(classification.name == "Invoices")
    #expect(classification.color == "#123456")
    #expect(classification.actorId == nil)
    #expect(classification.causationId == "24201525-7afd-4684-baea-eb1ed333c209")
}
