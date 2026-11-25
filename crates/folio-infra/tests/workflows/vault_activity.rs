use super::*;
use folio_auth::User;
use folio_core::models::{
    Principal, UserId, VaultId, VaultRole, WebhookPayload, WebhookVaultActivity,
    WorkflowActorCondition, WorkflowEventConditions,
};
use folio_core::mutation::with_actor;
use folio_core::{DomainEventEnvelope, EventAudience};
use folio_infra::Repositories;

#[tokio::test]
async fn sqlite_dispatches_vault_activity_within_live_owner_scope() {
    let (_directory, database) = common::sqlite_database().await;
    contract(&database).await;
}

#[tokio::test]
async fn postgres_dispatches_vault_activity_within_live_owner_scope() {
    let Some((_container, database)) = common::postgres_database("vault activity workflows").await
    else {
        return;
    };
    contract(&database).await;
}

async fn occurrence(repos: &Repositories, vault: VaultId, name: &str) -> OutboxEvent {
    let event = repos
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.vault_id() == vault && envelope.event_type() == name
            })
        })
        .unwrap();
    assert_eq!(
        event.envelope.as_ref().unwrap().audience(),
        &EventAudience::Owners
    );
    assert!(event.event.is_none());
    event
}

async fn register(
    repos: &Repositories,
    vault: VaultId,
    trigger: WorkflowTrigger,
    actor: Option<UserId>,
    server: &TestServer,
) {
    repos
        .workflows
        .create(
            vault,
            WorkflowInput {
                name: trigger.label().into(),
                enabled: true,
                trigger,
                triggers: Vec::new(),
                cadence: None,
                conditions: WorkflowConditions {
                    event: actor.map(|user_id| WorkflowEventConditions {
                        actor: Some(WorkflowActorCondition::User { user_id }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                actions: vec![webhook(server.url(trigger.as_str()), None)],
            },
        )
        .await
        .unwrap();
}

async fn deliver(
    engine: &WorkflowEngine,
    runner: &WorkflowRunner,
    event: &OutboxEvent,
) -> WebhookVaultActivity {
    let envelope = event.envelope.as_ref().unwrap();
    let (first, second) = tokio::join!(engine.handle(event), engine.handle(event));
    let mut runs = first.unwrap();
    runs.extend(second.unwrap());
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert!(run.document_id.is_none());
    let payload: WebhookPayload = serde_json::from_str(&run.payload).unwrap();
    let activity = payload.vault_activity.unwrap();
    assert!(payload.document.is_none());
    assert_eq!(payload.source_event.unwrap().event_id, envelope.event_id());
    assert_eq!(payload.occurred_at, envelope.occurred_at());
    assert_eq!(activity.vault_id, envelope.vault_id());
    assert_eq!(activity.subject_id, envelope.subject().id);
    assert_eq!(activity.actor_id.as_ref(), envelope.actor_id());
    assert_eq!(activity.causation_id, envelope.causation_id());
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    activity
}

async fn contract(database: &Database) {
    let repos = database.repositories().await;
    let owner = UserId::from("activity-owner");
    let member = UserId::from("activity-member");
    for user in [&owner, &member] {
        repos
            .auth
            .create_user(User {
                id: user.to_string(),
                username: user.to_string(),
                password_hash: "!".into(),
                created_at: 1,
                email: None,
                display_name: None,
                is_admin: false,
                is_owner: false,
                auth_source: "local".into(),
                external_id: None,
            })
            .await
            .unwrap();
    }
    repos.vaults.instance_default().await.unwrap();
    let vault = repos
        .vaults
        .create("Activity", Some(&owner))
        .await
        .unwrap()
        .id;
    let ownerless = repos
        .vaults
        .create("Ownerless archive", None)
        .await
        .unwrap()
        .id;
    let outside = repos
        .vaults
        .create("Other owned vault", Some(&owner))
        .await
        .unwrap()
        .id;
    let (engine, runner) = vault_resource_workflows(&repos);
    let server = TestServer::start().await;
    use WorkflowTrigger::*;
    for trigger in [
        VaultUpdated,
        VaultRestored,
        MembershipGranted,
        MembershipRoleChanged,
        MembershipRevoked,
    ] {
        register(&repos, vault, trigger, Some(owner.clone()), &server).await;
    }
    register(&repos, ownerless, VaultUpdated, None, &server).await;
    register(
        &repos,
        outside,
        MembershipRevoked,
        Some(owner.clone()),
        &server,
    )
    .await;
    let created = occurrence(&repos, vault, "vault.created").await;
    assert!(engine.handle(&created).await.unwrap().is_empty());

    repos
        .vaults
        .rename(ownerless, "Ownerless renamed")
        .await
        .unwrap();
    let ownerless_event = occurrence(&repos, ownerless, "vault.updated").await;
    assert!(engine.handle(&ownerless_event).await.unwrap().is_empty());

    with_actor(
        Some(owner.clone()),
        exercise(
            &repos,
            &engine,
            &runner,
            &server,
            vault,
            outside,
            Principal::User(member),
        ),
    )
    .await;

    with_actor(
        Some(owner.clone()),
        repos.vaults.rename(vault, "Before owner loss"),
    )
    .await
    .unwrap();
    let late = occurrence(&repos, vault, "vault.updated").await;
    let run = engine.handle(&late).await.unwrap().pop().unwrap();
    remove_last_owner_for_test(database, vault, &owner).await;
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    let failed = repos.workflows.find_run(run.id).await.unwrap().unwrap();
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Vault activity is no longer authorized")
    );
    assert_eq!(server.hits("vault.updated"), 1);
}

async fn exercise(
    repos: &Repositories,
    engine: &WorkflowEngine,
    runner: &WorkflowRunner,
    server: &TestServer,
    vault: VaultId,
    outside: VaultId,
    principal: Principal,
) {
    repos.vaults.rename(vault, "First title").await.unwrap();
    let updated = occurrence(repos, vault, "vault.updated").await;
    repos.vaults.rename(vault, "Second title").await.unwrap();
    let pending_rename = occurrence(repos, vault, "vault.updated").await;
    let activity = deliver(engine, runner, &updated).await;
    assert_eq!(activity.name.as_deref(), Some("First title"));

    repos
        .members
        .grant(vault, &principal, VaultRole::Viewer, None)
        .await
        .unwrap();
    let grant = occurrence(repos, vault, "vault_membership.granted").await;
    repos
        .members
        .grant(vault, &principal, VaultRole::Editor, None)
        .await
        .unwrap();
    let changed = occurrence(repos, vault, "vault_membership.role_changed").await;
    repos.members.revoke(vault, &principal).await.unwrap();
    let revoked = occurrence(repos, vault, "vault_membership.revoked").await;
    repos
        .members
        .grant(vault, &principal, VaultRole::Contributor, None)
        .await
        .unwrap();

    let envelope = revoked.envelope.as_ref().unwrap();
    let invalid = DomainEventEnvelope::new(
        "vault_membership.revoked",
        outside,
        envelope.subject().clone(),
        None,
        EventAudience::Owners,
        envelope.data().clone(),
    )
    .unwrap();
    let mut wrong_vault = revoked.clone();
    wrong_vault.envelope = Some(invalid);
    assert!(engine.handle(&wrong_vault).await.unwrap().is_empty());
    let mut from_workflow = revoked.clone();
    from_workflow.origin = EventOrigin::Workflow;
    assert!(engine.handle(&from_workflow).await.unwrap().is_empty());

    let subject = Id::new_v5(&vault.0, b"user:activity-member");
    for (event, before, after) in [
        (grant, None, Some(VaultRole::Viewer)),
        (changed, Some(VaultRole::Viewer), Some(VaultRole::Editor)),
        (revoked, Some(VaultRole::Editor), None),
    ] {
        let activity = deliver(engine, runner, &event).await;
        assert_eq!(activity.subject_id, subject);
        assert!(activity.name.is_none());
        let membership = activity.membership.unwrap();
        assert_eq!(membership.principal_type, "user");
        assert_eq!(membership.principal_id, "activity-member");
        assert_eq!((membership.before_role, membership.role), (before, after));
    }

    let run = engine.handle(&pending_rename).await.unwrap().pop().unwrap();
    repos.vaults.delete(vault).await.unwrap();
    let deleted = occurrence(repos, vault, "vault.deleted").await;
    assert!(engine.handle(&deleted).await.unwrap().is_empty());
    assert!(engine.handle(&pending_rename).await.unwrap().is_empty());
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    repos.vaults.restore(vault).await.unwrap();
    let restored = occurrence(repos, vault, "vault.restored").await;
    let activity = deliver(engine, runner, &restored).await;
    assert_eq!(activity.name.as_deref(), Some("Second title"));
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    assert_eq!(server.state.received.lock().unwrap().len(), 5);
}
