use super::*;
use folio_core::models::{
    CreateDocumentType, UpdateCorrespondent, UpdateDocumentType, UpdateTag, VaultId,
    WebhookPayload, WorkflowActorCondition, WorkflowEventConditions, WorkflowTriggerRegistry,
};
use folio_core::{DomainEventEnvelope, EventAudience};
use folio_infra::Repositories;

#[tokio::test]
async fn sqlite_delivers_only_scoped_classification_occurrences() {
    let (_directory, database) = common::sqlite_database().await;
    classification_contract(database.repositories().await).await;
}

#[tokio::test]
async fn postgres_delivers_only_scoped_classification_occurrences() {
    let Some((_container, database)) =
        common::postgres_database("classification workflow events").await
    else {
        return;
    };
    classification_contract(database.repositories().await).await;
}

async fn latest(repos: &Repositories, id: Id, trigger: WorkflowTrigger) -> OutboxEvent {
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
    event
}

async fn deliver(
    engine: &WorkflowEngine,
    runner: &WorkflowRunner,
    event: &OutboxEvent,
    name: &str,
) -> WebhookPayload {
    let envelope = event.envelope.as_ref().unwrap();
    let (first, second) = tokio::join!(engine.handle(event), engine.handle(event));
    let mut runs = first.unwrap();
    runs.extend(second.unwrap());
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert!(run.document_id.is_none());
    let payload: WebhookPayload = serde_json::from_str(&run.payload).unwrap();
    let classification = payload.classification.as_ref().unwrap();
    assert_eq!(classification.name, name);
    assert_eq!(classification.vault_id, envelope.vault_id());
    assert_eq!(classification.subject_id, envelope.subject().id);
    assert_eq!(classification.actor_id.as_ref(), envelope.actor_id());
    assert_eq!(classification.causation_id, envelope.causation_id());
    assert_eq!(payload.occurred_at, envelope.occurred_at());
    assert_eq!(payload.source_event.unwrap().event_id, envelope.event_id());
    assert!(payload.document.is_none());
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    payload
}

async fn configure(repos: &Repositories, vault: VaultId, server: &TestServer) {
    for definition in WorkflowTriggerRegistry::built_in().definitions() {
        let trigger = WorkflowTrigger::parse(&definition.event_type).unwrap();
        if trigger.classification_subject().is_none() {
            continue;
        }
        repos
            .workflows
            .create(
                vault,
                WorkflowInput {
                    name: format!("Listen to {}", definition.event_type),
                    enabled: true,
                    trigger,
                    triggers: Vec::new(),
                    cadence: None,
                    conditions: WorkflowConditions {
                        event: Some(WorkflowEventConditions {
                            actor: Some(WorkflowActorCondition::System),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    actions: vec![webhook(server.url(&definition.event_type), None)],
                },
            )
            .await
            .unwrap();
    }
}

async fn classification_contract(repos: Repositories) {
    let vault = repos.vaults.instance_default().await.unwrap();
    let (engine, runner) = vault_resource_workflows(&repos);
    let server = TestServer::start().await;
    configure(&repos, vault, &server).await;
    use WorkflowTrigger::*;

    let tag = repos
        .tags
        .create(
            vault,
            CreateTag {
                name: "Invoices".into(),
                color: Some("#123456".into()),
            },
        )
        .await
        .unwrap();
    let creation = latest(&repos, tag.id, TagCreated).await;
    let envelope = creation.envelope.as_ref().unwrap();
    let mut restricted = creation.clone();
    restricted.envelope = Some(
        DomainEventEnvelope::new(
            TagCreated.as_str(),
            vault,
            envelope.subject().clone(),
            None,
            EventAudience::User {
                user_id: "restricted".into(),
            },
            envelope.data().clone(),
        )
        .unwrap(),
    );
    assert!(engine.handle(&restricted).await.unwrap().is_empty());
    let mut from_workflow = creation.clone();
    from_workflow.origin = EventOrigin::Workflow;
    assert!(engine.handle(&from_workflow).await.unwrap().is_empty());

    let other = repos.vaults.create("Outside", None).await.unwrap().id;
    configure(&repos, other, &server).await;
    let mut wrong_vault = creation.clone();
    let mut moved_data = envelope.data().clone();
    moved_data["vault_id"] = serde_json::json!(other);
    wrong_vault.envelope = Some(
        DomainEventEnvelope::new(
            TagCreated.as_str(),
            other,
            envelope.subject().clone(),
            None,
            EventAudience::Vault,
            moved_data,
        )
        .unwrap(),
    );
    assert!(engine.handle(&wrong_vault).await.unwrap().is_empty());

    repos
        .tags
        .update(
            vault,
            tag.id,
            UpdateTag {
                name: Some("Receipts".into()),
                color: Some(None),
            },
        )
        .await
        .unwrap();
    let payload = deliver(&engine, &runner, &creation, "Invoices").await;
    assert_eq!(
        payload.classification.unwrap().color.as_deref(),
        Some("#123456")
    );
    let tag_updated = latest(&repos, tag.id, TagUpdated).await;
    let payload = deliver(&engine, &runner, &tag_updated, "Receipts").await;
    assert!(payload.classification.unwrap().color.is_none());

    repos.tags.delete(vault, tag.id).await.unwrap();
    assert!(engine.handle(&creation).await.unwrap().is_empty());
    let tag_deleted = latest(&repos, tag.id, TagDeleted).await;
    deliver(&engine, &runner, &tag_deleted, "Receipts").await;
    repos.tags.restore(vault, tag.id).await.unwrap();
    let tag_restored = latest(&repos, tag.id, TagRestored).await;
    deliver(&engine, &runner, &tag_restored, "Receipts").await;

    let correspondent = repos
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Sender".into(),
            },
        )
        .await
        .unwrap();
    let id = correspondent.id;
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, CorrespondentCreated).await,
        "Sender",
    )
    .await;
    repos
        .correspondents
        .update(
            vault,
            id,
            UpdateCorrespondent {
                name: Some("New sender".into()),
            },
        )
        .await
        .unwrap();
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, CorrespondentUpdated).await,
        "New sender",
    )
    .await;
    repos.correspondents.delete(vault, id).await.unwrap();
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, CorrespondentDeleted).await,
        "New sender",
    )
    .await;
    repos.correspondents.restore(vault, id).await.unwrap();
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, CorrespondentRestored).await,
        "New sender",
    )
    .await;

    let document_type = repos
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Statement".into(),
            },
        )
        .await
        .unwrap();
    let id = document_type.id;
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, DocumentTypeCreated).await,
        "Statement",
    )
    .await;
    repos
        .document_types
        .update(
            vault,
            id,
            UpdateDocumentType {
                name: Some("Account statement".into()),
            },
        )
        .await
        .unwrap();
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, DocumentTypeUpdated).await,
        "Account statement",
    )
    .await;
    repos.document_types.delete(vault, id).await.unwrap();
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, DocumentTypeDeleted).await,
        "Account statement",
    )
    .await;
    repos.document_types.restore(vault, id).await.unwrap();
    deliver(
        &engine,
        &runner,
        &latest(&repos, id, DocumentTypeRestored).await,
        "Account statement",
    )
    .await;

    repos
        .tags
        .update(
            vault,
            tag.id,
            UpdateTag {
                name: Some("Before deletion".into()),
                color: None,
            },
        )
        .await
        .unwrap();
    let stale = latest(&repos, tag.id, TagUpdated).await;
    let run = engine.handle(&stale).await.unwrap().pop().unwrap();
    repos.tags.delete(vault, tag.id).await.unwrap();
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    assert_eq!(server.hits(TagUpdated.as_str()), 1);
    assert_eq!(server.state.received.lock().unwrap().len(), 12);
}
