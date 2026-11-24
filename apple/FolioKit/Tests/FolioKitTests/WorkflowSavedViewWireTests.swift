import FolioAPI
import Foundation
import Testing

@Test func sharedSavedViewWebhookDecodesWithoutADocument() throws {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    let payload = try decoder.decode(
        Components.Schemas.WebhookPayload.self,
        from: Data(
            """
            {"version":1,"event":"saved_view.updated","occurred_at":"2026-11-23T14:00:00Z",
            "saved_view":{"vault_id":"0e72ce5f-74dd-4471-8a87-beb383526a3a",
            "saved_view_id":"3fa85f64-5717-4562-b3fc-2c963f66afa6",
            "name":"Invoices","presentation":"grid","visibility":"shared",
            "actor_id":"owner","causation_id":"24201525-7afd-4684-baea-eb1ed333c209"}}
            """.utf8)
    )
    let view = try #require(payload.savedView)
    #expect(payload.event.rawValue == "saved_view.updated")
    #expect(view.name == "Invoices")
    #expect(view.visibility.rawValue == "shared")
    #expect(view.presentation.rawValue == "grid")
    #expect(view.actorId == "owner")
    #expect(view.causationId == "24201525-7afd-4684-baea-eb1ed333c209")
}

@Test func savedViewChoicesMapToGeneratedWorkflowTriggers() throws {
    let decoder = JSONDecoder()
    for action in ["created", "updated", "deleted"] {
        let event = "saved_view.\(action)"
        let choice = try decoder.decode(
            Components.Schemas.WorkflowTriggerDefinition.self,
            from: Data(
                """
                {"event_type":"\(event)","label":"Shared saved view","subject_type":"saved_view",
                "target":"event","condition_fields":[{"key":"event.subject_id",
                    "label":"Subject ID","value_type":"identifier"}]}
                """.utf8)
        )
        #expect(Components.Schemas.WorkflowTrigger(rawValue: choice.eventType)?.rawValue == event)
        #expect(choice.target.rawValue == "event")
        #expect(choice.conditionFields.first?.key == "event.subject_id")
    }
}
