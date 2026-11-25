mod common;

use chrono::{Duration, Utc};
use folio_core::domain_event::EventAudience;
use folio_core::events::EventOrigin;
use folio_core::models::{
    RunStatus, VaultId, WebhookMethod, Workflow, WorkflowAction, WorkflowEventIdentity,
    WorkflowInput, WorkflowRun, WorkflowTrigger,
};
use folio_core::ports::OutboxStore;
use folio_infra::{Database, PostgresOutboxStore, SqliteOutboxStore};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    let workflows = repositories.workflows;
    let vault = repositories.vaults.instance_default().await.unwrap();
    let input: WorkflowInput = serde_json::from_value(serde_json::json!({
        "name": "Outcome contract",
        "enabled": true,
        "trigger": WorkflowTrigger::DocumentUploaded,
        "actions": [WorkflowAction::Webhook {
            url: "https://example.test".into(),
            method: WebhookMethod::Post,
            headers: Default::default(),
            secret: Some("private webhook secret".into()),
        }],
    }))
    .unwrap();
    let definition = workflows.create(vault, input.clone()).await.unwrap();
    let outbox: Box<dyn OutboxStore> = match &db {
        Database::Sqlite(pool) => Box::new(SqliteOutboxStore::new(pool.clone())),
        Database::Postgres(pool) => Box::new(PostgresOutboxStore::new(pool.clone())),
    };
    let source = WorkflowEventIdentity {
        event_id: Uuid::new_v4(),
        mutation_id: Uuid::new_v4(),
        correlation_id: Uuid::new_v4(),
    };
    let new_run = |definition: &Workflow, identity| {
        WorkflowRun::new(
            definition.vault_id,
            definition.id,
            definition.trigger,
            Some(Uuid::new_v4()),
            serde_json::json!({"source_event": identity, "private": "captured text"}).to_string(),
        )
        .with_definition(definition)
    };
    let mut run = new_run(&definition, Some(source));
    assert!(workflows.create_run(&run).await.unwrap());
    run.attempts = 1;
    run.last_error = Some("temporary endpoint error".into());
    workflows.update_run(&run).await.unwrap();
    assert!(outbox
        .claim(50, Duration::seconds(30), 10)
        .await
        .unwrap()
        .iter()
        .all(|event| event
            .envelope
            .as_ref()
            .is_none_or(|envelope| { !envelope.event_type().starts_with("workflow_run.") })));

    run.status = RunStatus::Succeeded;
    run.completed_actions = 1;
    run.response_status = Some(200);
    run.finished_at = Some(Utc::now());
    run.vault_id = VaultId::new();
    run.workflow_id = Uuid::new_v4();
    run.payload = "{}".into();
    let (first, second) = tokio::join!(workflows.update_run(&run), workflows.update_run(&run));
    assert!(matches!(
        (first, second),
        (Ok(()), Err(folio_core::Error::Conflict(_)))
            | (Err(folio_core::Error::Conflict(_)), Ok(()))
    ));

    let mut failure = new_run(&definition, None);
    assert!(workflows.create_run(&failure).await.unwrap());
    failure.status = RunStatus::Failed;
    failure.finished_at = Some(Utc::now());
    failure.last_error = Some("private response".into());
    failure.response_status = Some(503);
    install_failure_guard(&db).await;
    reject_failures(&db, true).await;
    assert!(workflows.update_run(&failure).await.is_err());
    assert_eq!(
        workflows
            .find_run(failure.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Pending
    );
    reject_failures(&db, false).await;
    workflows.update_run(&failure).await.unwrap();

    let superseded = new_run(&definition, None);
    assert!(workflows.create_run(&superseded).await.unwrap());
    let update = WorkflowInput {
        name: "Revised definition".into(),
        ..input.clone()
    };
    reject_failures(&db, true).await;
    assert!(workflows
        .update(vault, definition.id, update.clone())
        .await
        .is_err());
    assert_eq!(
        workflows
            .find_run(superseded.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Pending
    );
    assert_eq!(
        workflows
            .find_by_id(vault, definition.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        definition.revision
    );
    reject_failures(&db, false).await;
    let edited = workflows
        .update(vault, definition.id, update)
        .await
        .unwrap();
    let stopped = new_run(&edited, None);
    assert!(workflows.create_run(&stopped).await.unwrap());
    workflows.delete(vault, edited.id).await.unwrap();
    workflows.delete(vault, edited.id).await.unwrap();

    let extra_vault = repositories
        .vaults
        .create("Deleted vault", None)
        .await
        .unwrap()
        .id;
    let extra_definition = workflows.create(extra_vault, input).await.unwrap();
    let vault_stopped = new_run(&extra_definition, None);
    assert!(workflows.create_run(&vault_stopped).await.unwrap());
    repositories.vaults.delete(extra_vault).await.unwrap();
    repositories.vaults.delete(extra_vault).await.unwrap();

    let claimed = outbox.claim(50, Duration::seconds(30), 10).await.unwrap();
    let outcomes: Vec<_> = claimed
        .iter()
        .filter_map(|event| {
            let envelope = event.envelope.as_ref()?;
            envelope.event_type().starts_with("workflow_run.").then(|| {
                assert_eq!(event.origin, EventOrigin::Workflow);
                assert_eq!(envelope.audience(), &EventAudience::Owners);
                let definition = if envelope.subject().id == vault_stopped.id {
                    &extra_definition
                } else {
                    &definition
                };
                assert_eq!(envelope.vault_id(), definition.vault_id);
                assert_eq!(envelope.subject().subject_type, "workflow_run");
                assert_eq!(envelope.document_id(), None);
                assert_eq!(envelope.mutation_id(), envelope.subject().id);
                assert_eq!(
                    envelope.data(),
                    &serde_json::json!({"workflow_id": definition.id})
                );
                envelope
            })
        })
        .collect();
    assert_eq!(outcomes.len(), 5);
    let succeeded = outcomes
        .iter()
        .find(|event| event.subject().id == run.id)
        .unwrap();
    assert_eq!(succeeded.event_type(), "workflow_run.succeeded");
    assert_eq!(succeeded.correlation_id(), source.correlation_id);
    assert_eq!(succeeded.causation_id(), Some(source.event_id));
    for finished in [failure, superseded, stopped, vault_stopped] {
        let outcome = outcomes
            .iter()
            .find(|event| event.subject().id == finished.id)
            .unwrap();
        assert_eq!(outcome.event_type(), "workflow_run.failed");
        assert_eq!(outcome.correlation_id(), finished.id);
        assert_eq!(outcome.causation_id(), None);
        assert_eq!(
            workflows
                .find_run(finished.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            RunStatus::Failed
        );
    }
}

async fn install_failure_guard(db: &Database) {
    // A second SQLite connection can cache the previously dropped trigger name.
    match db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql(
                "CREATE TABLE workflow_outcome_fault (active BOOLEAN NOT NULL);
                 INSERT INTO workflow_outcome_fault VALUES (FALSE);
                 CREATE TRIGGER reject_run_failure BEFORE INSERT ON outbox_events
                 WHEN NEW.type = 'workflow_run.failed' AND (SELECT active FROM workflow_outcome_fault)
                 BEGIN
                   SELECT RAISE(ABORT, 'rejected workflow outcome');
                 END;",
            )
            .execute(pool)
            .await
            .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql(
                "CREATE TABLE workflow_outcome_fault (active BOOLEAN NOT NULL);
                 INSERT INTO workflow_outcome_fault VALUES (FALSE);
                 CREATE FUNCTION reject_run_failure() RETURNS trigger LANGUAGE plpgsql
                 AS $$ BEGIN
                   IF (SELECT active FROM workflow_outcome_fault) THEN
                     RAISE EXCEPTION 'rejected workflow outcome';
                   END IF;
                   RETURN NEW;
                 END; $$;
                 CREATE TRIGGER reject_run_failure BEFORE INSERT ON outbox_events
                 FOR EACH ROW WHEN (NEW.type = 'workflow_run.failed')
                 EXECUTE FUNCTION reject_run_failure();",
            )
            .execute(pool)
            .await
            .unwrap();
        }
    }
}

async fn reject_failures(db: &Database, active: bool) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE workflow_outcome_fault SET active = ?")
                .bind(active)
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE workflow_outcome_fault SET active = $1")
                .bind(active)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn sqlite_workflow_run_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("workflow-runs.db").display());
    contract(Database::connect(&url).await.unwrap()).await;
}

#[tokio::test]
async fn postgres_workflow_run_outcomes() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow outcomes",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    contract(Database::connect(&url).await.unwrap()).await;
}
