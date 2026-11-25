import FolioAPI
import Foundation
import Testing

@Test func generatedVaultActivityRetainsMembershipRoleChanges() throws {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    let payload = try decoder.decode(
        Components.Schemas.WebhookPayload.self,
        from: Data(
            """
            {"event":"vault_membership.role_changed","version":1,"occurred_at":"2026-11-23T14:00:00Z",
            "vault_activity":{"vault_id":"0e72ce5f-74dd-4471-8a87-beb383526a3a",
            "subject_id":"067da63b-4127-5d04-9b65-2ba26a0289c5","actor_id":"owner",
            "membership":{"principal_type":"group","principal_id":"finance",
            "before_role":"viewer","role":"editor"}}}
            """.utf8)
    )
    let activity = try #require(payload.vaultActivity)
    let membership = try #require(activity.membership)
    #expect(payload.event.rawValue == "vault_membership.role_changed")
    #expect(activity.actorId == "owner")
    #expect(membership.principalType == "group")
    #expect(membership.principalId == "finance")
    #expect(membership.beforeRole == "viewer")
    #expect(membership.role == "editor")
}

@Test func generatedTriggerChoicesCoverConsumableVaultActivity() {
    for event in [
        "vault.updated", "vault.restored", "vault_membership.granted",
        "vault_membership.role_changed", "vault_membership.revoked",
    ] {
        #expect(Components.Schemas.WorkflowTrigger(rawValue: event) != nil)
    }
}
