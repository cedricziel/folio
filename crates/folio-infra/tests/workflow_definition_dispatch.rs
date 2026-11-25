mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use folio_auth::User;
use folio_core::models::{
    Id, Principal, RunStatus, UserId, VaultId, VaultRole, Workflow, WorkflowAction, WorkflowInput,
    WorkflowRun,
};
use folio_core::mutation::with_actor;
use folio_core::ports::{OutboxStore, WorkflowRepository};
use folio_core::services::{IngestWorkflowAccess, WorkflowEngine};
use folio_core::{DomainEventEnvelope, EventAudience, EventOrigin, OutboxEvent};
use folio_infra::{
    workflow::RunOutcome, Database, ObjectStoreDocumentStore, Repositories, StorageFileMetadata,
    WebhookClient, WorkflowRunner,
};
use serde_json::{json, Value};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

fn definition(name: &str, enabled: bool, trigger: &str, subject: Option<Id>) -> WorkflowInput {
    let conditions = subject.map_or_else(|| json!({}), |id| json!({"event": {"subject_id": id}}));
    serde_json::from_value(json!({
        "name": name,
        "enabled": enabled,
        "trigger": trigger,
        "conditions": conditions,
        "actions": [{"type": "webhook", "url": "https://example.test/notify"}],
    }))
    .unwrap()
}

async fn listener(
    workflows: &dyn WorkflowRepository,
    vault: VaultId,
    target: Id,
    trigger: &str,
) -> Workflow {
    workflows
        .create(
            vault,
            definition("Notify the owner", true, trigger, Some(target)),
        )
        .await
        .unwrap()
}

async fn claim(outbox: &dyn OutboxStore, target: Id) -> Vec<OutboxEvent> {
    outbox
        .claim(100, Duration::seconds(30), 10)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|source| source.subject().id == target)
        })
        .collect()
}

fn event(events: &[OutboxEvent], kind: &str) -> OutboxEvent {
    events
        .iter()
        .find(|event| event.envelope.as_ref().unwrap().event_type() == kind)
        .unwrap()
        .clone()
}

fn access(repos: &Repositories, auth_required: bool) -> Arc<IngestWorkflowAccess> {
    Arc::new(IngestWorkflowAccess::new(
        repos.sources.clone(),
        repos.vaults.clone(),
        repos.members.clone(),
        &[],
        auth_required,
    ))
}

fn engine(repos: &Repositories, access: Arc<IngestWorkflowAccess>) -> WorkflowEngine {
    WorkflowEngine::new(
        repos.workflows.clone(),
        repos.documents.clone(),
        repos.correspondents.clone(),
        repos.document_types.clone(),
        Arc::new(StorageFileMetadata::new(Arc::new(
            ObjectStoreDocumentStore::in_memory(),
        ))),
        "http://folio.test",
    )
    .with_ingest_access(access)
}

async fn verify_definition_dispatch(db: Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let other_vault = repos
        .vaults
        .create("Other owner's vault", None)
        .await
        .unwrap()
        .id;
    let workflows = &*repos.workflows;
    let source = workflows
        .create(
            vault,
            definition("Original name", true, "document.uploaded", None),
        )
        .await
        .unwrap();
    let created = listener(workflows, vault, source.id, "workflow.created").await;
    let updated = listener(workflows, vault, source.id, "workflow.updated").await;
    let disabled = listener(workflows, vault, source.id, "workflow.disabled").await;
    let enabled = listener(workflows, vault, source.id, "workflow.enabled").await;
    let restored = listener(workflows, vault, source.id, "workflow.restored").await;
    let deleted = listener(workflows, vault, source.id, "workflow.deleted").await;
    let wrong_vault = listener(workflows, other_vault, source.id, "workflow.disabled").await;
    with_actor(
        Some("owner-at-occurrence".into()),
        workflows.update(
            vault,
            source.id,
            definition("Renamed", false, "document.processed", None),
        ),
    )
    .await
    .unwrap();
    workflows
        .update(
            vault,
            source.id,
            definition("Resumed", true, "document.processed", None),
        )
        .await
        .unwrap();
    workflows.delete(vault, source.id).await.unwrap();

    let engine = engine(&repos, access(&repos, false));
    let events = claim(&*repos.outbox, source.id).await;
    let created_run = engine
        .handle(&event(&events, "workflow.created"))
        .await
        .unwrap();
    assert_eq!(
        created_run
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [created.id]
    );
    let created_body: Value = serde_json::from_str(&created_run[0].payload).unwrap();
    assert_eq!(created_body["workflow"]["name"], "Original name");
    assert_eq!(created_body["workflow"]["trigger"], "document.uploaded");
    assert_eq!(created_body["workflow"]["enabled"], true);

    let disabling = event(&events, "workflow.disabled");
    assert!(engine
        .handle(&OutboxEvent {
            origin: EventOrigin::Workflow,
            ..disabling.clone()
        })
        .await
        .unwrap()
        .is_empty());
    let owner_occurrence = disabling.envelope.as_ref().unwrap();
    let private_occurrence = DomainEventEnvelope::new(
        owner_occurrence.event_type(),
        vault,
        owner_occurrence.subject().clone(),
        None,
        EventAudience::User {
            user_id: "private-recipient".into(),
        },
        owner_occurrence.data().clone(),
    )
    .unwrap();
    assert!(engine
        .handle(&OutboxEvent {
            envelope: Some(private_occurrence),
            ..disabling.clone()
        })
        .await
        .unwrap()
        .is_empty());
    let disabled_run = engine.handle(&disabling).await.unwrap();
    assert_eq!(
        disabled_run
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [disabled.id]
    );
    let body: Value = serde_json::from_str(&disabled_run[0].payload).unwrap();
    assert_eq!(body["workflow"]["id"], json!(source.id));
    assert_eq!(body["workflow"]["vault_id"], json!(vault));
    assert_eq!(body["workflow"]["actor_id"], "owner-at-occurrence");
    assert_eq!(body["workflow"]["enabled"], false);
    assert_eq!(body["workflow"]["name"], "Renamed");
    assert!(body["workflow"].get("actions").is_none());
    assert!(disabled_run[0].document_id.is_none());
    assert!(engine.handle(&disabling).await.unwrap().is_empty());
    assert!(workflows
        .list_runs(other_vault, wrong_vault.id, 10)
        .await
        .unwrap()
        .is_empty());

    let updating = event(&events, "workflow.updated");
    assert_eq!(
        updating.envelope.as_ref().unwrap().mutation_id(),
        owner_occurrence.mutation_id()
    );
    let updated_runs = engine.handle(&updating).await.unwrap();
    let enabled_runs = engine
        .handle(&event(&events, "workflow.enabled"))
        .await
        .unwrap();
    assert_eq!(
        updated_runs
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [updated.id]
    );
    assert_eq!(
        enabled_runs
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [enabled.id]
    );

    let deleted_run = engine
        .handle(&event(&events, "workflow.deleted"))
        .await
        .unwrap();
    assert_eq!(
        deleted_run
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [deleted.id]
    );
    workflows.restore(vault, source.id).await.unwrap();
    let restored_events = claim(&*repos.outbox, source.id).await;
    let restored_runs = engine
        .handle(&event(&restored_events, "workflow.restored"))
        .await
        .unwrap();
    assert_eq!(
        restored_runs
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [restored.id]
    );
    verify_owner_loss(&db, &repos, vault).await;
}

async fn verify_owner_loss(db: &Database, repos: &Repositories, vault: VaultId) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let owner = UserId::from("current-workflow-owner");
    repos
        .auth
        .create_user(User {
            id: owner.to_string(),
            username: owner.to_string(),
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
    assert_eq!(repos.members.direct_owner_count(vault).await.unwrap(), 0);
    let authenticated = access(repos, true);
    let engine = engine(repos, authenticated.clone());
    let runner = WorkflowRunner::new(
        repos.workflows.clone(),
        repos.workflow_documents.clone(),
        WebhookClient::new(),
    )
    .with_ingest_access(authenticated);

    let source = repos
        .workflows
        .create(
            vault,
            definition("Owner-managed definition", true, "document.uploaded", None),
        )
        .await
        .unwrap();
    for trigger in ["workflow.updated", "workflow_run.succeeded"] {
        let mut input = definition("Notify the current owner", true, trigger, None);
        let WorkflowAction::Webhook { url, .. } = &mut input.actions[0] else {
            unreachable!("the definition contains one webhook")
        };
        *url = format!("{}/workflow-notification", server.uri());
        repos.workflows.create(vault, input).await.unwrap();
    }

    let before = definition("First edit", true, "document.uploaded", None);
    repos
        .workflows
        .update(vault, source.id, before)
        .await
        .unwrap();
    let before = event(&claim(&*repos.outbox, source.id).await, "workflow.updated");
    assert!(engine.handle(&before).await.unwrap().is_empty());
    repos
        .members
        .grant(
            vault,
            &Principal::User(owner.clone()),
            VaultRole::Owner,
            None,
        )
        .await
        .unwrap();

    let definition_run = engine.handle(&before).await.unwrap().pop().unwrap();
    let current = repos
        .workflows
        .find_by_id(vault, source.id)
        .await
        .unwrap()
        .unwrap();
    let mut source_run = WorkflowRun::new(vault, source.id, source.trigger, None, "{}".into())
        .with_definition(&current);
    assert!(repos.workflows.create_run(&source_run).await.unwrap());
    source_run.status = RunStatus::Succeeded;
    source_run.finished_at = Some(Utc::now());
    repos.workflows.update_run(&source_run).await.unwrap();
    let outcome = event(
        &claim(&*repos.outbox, source_run.id).await,
        "workflow_run.succeeded",
    );
    let outcome_run = engine.handle(&outcome).await.unwrap().pop().unwrap();

    // The repository normally rejects removal of the last direct owner.
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DELETE FROM vault_members WHERE vault_id = ? AND user_id = ?")
                .bind(vault.to_string())
                .bind(owner.as_str())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DELETE FROM vault_members WHERE vault_id = $1 AND user_id = $2")
                .bind(vault.0)
                .bind(owner.as_str())
                .execute(pool)
                .await
                .unwrap();
        }
    }
    for run in [definition_run, outcome_run] {
        assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
        let stored = repos.workflows.find_run(run.id).await.unwrap().unwrap();
        assert_eq!(
            stored.last_error.as_deref(),
            Some("Workflow event is no longer authorized")
        );
        assert_eq!(stored.completed_actions, 0);
    }
    assert!(server.received_requests().await.unwrap().is_empty());

    repos
        .workflows
        .update(
            vault,
            source.id,
            definition("Second edit", true, "document.uploaded", None),
        )
        .await
        .unwrap();
    let after = event(&claim(&*repos.outbox, source.id).await, "workflow.updated");
    assert!(engine.handle(&after).await.unwrap().is_empty());
}

#[tokio::test]
async fn sqlite_dispatches_captured_workflow_definition_events_in_the_owning_vault() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite:{}",
        dir.path().join("workflow-definition-events.db").display()
    );
    verify_definition_dispatch(Database::connect(&url).await.unwrap()).await;
}

#[tokio::test]
async fn postgres_dispatches_captured_workflow_definition_events_in_the_owning_vault() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow definition dispatch",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    verify_definition_dispatch(Database::connect(&url).await.unwrap()).await;
}
