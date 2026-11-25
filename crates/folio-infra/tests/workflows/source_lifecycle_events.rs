use super::*;
use folio_auth::User;
use folio_core::models::{
    FolderAfterImport, FolderConfig, IngestionSourceInput, SourceConfig, SourceKind, VaultId,
    WebhookPayload,
};
use folio_core::{DomainEventEnvelope, EventAudience};
use folio_infra::Repositories;

#[tokio::test]
async fn sqlite_source_lifecycle_delivery_requires_current_owners_scope() {
    let (_directory, database) = common::sqlite_database().await;
    scope_contract(&database).await;
}

#[tokio::test]
async fn postgres_source_lifecycle_delivery_requires_current_owners_scope() {
    let Some((_container, database)) =
        common::postgres_database("managed-source workflow events").await
    else {
        return;
    };
    scope_contract(&database).await;
}

async fn occurrence(repos: &Repositories, id: Id, trigger: WorkflowTrigger) -> OutboxEvent {
    let event = repos
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.event_type() == trigger.as_str() && envelope.subject().id == id
            })
        })
        .unwrap();
    assert!(event.event.is_none());
    assert_eq!(
        event.envelope.as_ref().unwrap().audience(),
        &EventAudience::Owners
    );
    event
}

async fn verify_delivery(
    engine: &WorkflowEngine,
    runner: &WorkflowRunner,
    event: &OutboxEvent,
    enabled: bool,
    name: &str,
) {
    let envelope = event.envelope.as_ref().unwrap();
    let (first, second) = tokio::join!(engine.handle(event), engine.handle(event));
    let mut runs = first.unwrap();
    runs.extend(second.unwrap());
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert!(run.document_id.is_none());
    assert!(!run.payload.contains("folder-path-for-test-only"));
    let payload: WebhookPayload = serde_json::from_str(&run.payload).unwrap();
    assert_eq!(payload.source_event.unwrap().event_id, envelope.event_id());
    assert_eq!(payload.occurred_at, envelope.occurred_at());
    assert!(payload.document.is_none());
    assert!(payload.ingest.is_none());
    let source = payload.source.unwrap();
    assert_eq!(source.vault_id, envelope.vault_id());
    assert_eq!(source.source_id, envelope.subject().id);
    assert_eq!(source.kind, SourceKind::Folder);
    assert_eq!(source.enabled, enabled);
    assert_eq!(source.name, name);
    assert_eq!(source.actor_id.as_ref(), envelope.actor_id());
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
}

async fn configure(repos: &Repositories, vault: VaultId, server: &TestServer) {
    for trigger in [
        WorkflowTrigger::SourceCreated,
        WorkflowTrigger::SourceUpdated,
        WorkflowTrigger::SourceDeleted,
        WorkflowTrigger::SourceRestored,
    ] {
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
                    conditions: WorkflowConditions::default(),
                    actions: vec![webhook(server.url(trigger.as_str()), None)],
                },
            )
            .await
            .unwrap();
    }
}

async fn scope_contract(database: &Database) {
    use WorkflowTrigger::*;
    let repos = database.repositories().await;
    let owner = "managed-source-workflow-owner";
    repos
        .auth
        .create_user(User {
            id: owner.into(),
            username: owner.into(),
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
    repos.vaults.instance_default().await.unwrap();
    let vault = repos
        .vaults
        .create("Sources", Some(&owner.into()))
        .await
        .unwrap()
        .id;
    let ownerless = repos.vaults.create("No owner", None).await.unwrap().id;
    let (engine, runner) = vault_resource_workflows(&repos);
    let server = TestServer::start().await;
    configure(&repos, vault, &server).await;
    configure(&repos, ownerless, &server).await;

    let mut input = IngestionSourceInput {
        name: "Incoming docs".into(),
        enabled: true,
        config: SourceConfig::Folder(FolderConfig {
            path: "/folder-path-for-test-only".into(),
            recursive: false,
            subfolders_as_tags: false,
            after_import: FolderAfterImport::Delete,
            settle_secs: 0,
            rescan_secs: 60,
        }),
    };
    let without_owner = repos
        .sources
        .create(ownerless, input.clone())
        .await
        .unwrap();
    let restricted = occurrence(&repos, without_owner.id, SourceCreated).await;
    assert!(engine.handle(&restricted).await.unwrap().is_empty());
    let created = folio_core::mutation::with_actor(
        Some(owner.into()),
        repos.sources.create(vault, input.clone()),
    )
    .await
    .unwrap();
    let id = created.id;
    let creation = occurrence(&repos, id, SourceCreated).await;
    let envelope = creation.envelope.as_ref().unwrap();
    let outside = repos
        .vaults
        .create("Outside source vault", Some(&owner.into()))
        .await
        .unwrap()
        .id;
    configure(&repos, outside, &server).await;
    let mut wrong_vault = creation.clone();
    wrong_vault.envelope = Some(
        DomainEventEnvelope::new(
            SourceCreated.as_str(),
            outside,
            envelope.subject().clone(),
            None,
            EventAudience::Owners,
            envelope.data().clone(),
        )
        .unwrap(),
    );
    assert!(engine.handle(&wrong_vault).await.unwrap().is_empty());

    let mut broader_audience = creation.clone();
    broader_audience.envelope = Some(
        DomainEventEnvelope::new(
            SourceCreated.as_str(),
            vault,
            envelope.subject().clone(),
            None,
            EventAudience::Vault,
            envelope.data().clone(),
        )
        .unwrap(),
    );
    assert!(engine.handle(&broader_audience).await.unwrap().is_empty());
    let mut workflow_origin = creation.clone();
    workflow_origin.origin = EventOrigin::Workflow;
    assert!(engine.handle(&workflow_origin).await.unwrap().is_empty());

    input.name = "Disabled intake".into();
    input.enabled = false;
    repos
        .sources
        .update(vault, id, input.clone())
        .await
        .unwrap();
    verify_delivery(&engine, &runner, &creation, true, "Incoming docs").await;
    let update = occurrence(&repos, id, SourceUpdated).await;
    verify_delivery(&engine, &runner, &update, false, "Disabled intake").await;

    input.name = "Before trash".into();
    repos
        .sources
        .update(vault, id, input.clone())
        .await
        .unwrap();
    let late = occurrence(&repos, id, SourceUpdated).await;
    let run = engine.handle(&late).await.unwrap().pop().unwrap();
    repos.sources.delete(vault, id).await.unwrap();
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    assert!(engine.handle(&late).await.unwrap().is_empty());
    let deleted = occurrence(&repos, id, SourceDeleted).await;
    verify_delivery(&engine, &runner, &deleted, false, "Before trash").await;
    repos.sources.restore(vault, id).await.unwrap();
    let restored = occurrence(&repos, id, SourceRestored).await;
    verify_delivery(&engine, &runner, &restored, false, "Before trash").await;
    assert!(engine.handle(&deleted).await.unwrap().is_empty());

    input.name = "Before owner loss".into();
    repos
        .sources
        .update(vault, id, input.clone())
        .await
        .unwrap();
    let update = occurrence(&repos, id, SourceUpdated).await;
    let run = engine.handle(&update).await.unwrap().pop().unwrap();
    remove_last_owner_for_test(database, vault, &owner.into()).await;
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    let failed = repos.workflows.find_run(run.id).await.unwrap().unwrap();
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Managed source or workflow is no longer authorized")
    );

    input.name = "After owner loss".into();
    repos.sources.update(vault, id, input).await.unwrap();
    let update = occurrence(&repos, id, SourceUpdated).await;
    assert!(engine.handle(&update).await.unwrap().is_empty());
    assert_eq!(server.state.received.lock().unwrap().len(), 4);
}
