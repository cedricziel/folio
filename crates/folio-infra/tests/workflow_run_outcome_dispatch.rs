mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use folio_core::models::{RunStatus, WorkflowInput, WorkflowRun};
use folio_core::ports::OutboxStore;
use folio_core::services::{IngestWorkflowAccess, WorkflowEngine};
use folio_core::OutboxEvent;
use folio_infra::{Database, ObjectStoreDocumentStore, StorageFileMetadata};
use serde_json::{json, Value};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

fn listener(name: &str, trigger: &str) -> WorkflowInput {
    serde_json::from_value(json!({
        "name": name,
        "trigger": trigger,
        "actions": [{"type": "webhook", "url": "https://example.test/notify"}],
    }))
    .unwrap()
}

async fn claim_failure(outbox: &dyn OutboxStore, run_id: Uuid) -> OutboxEvent {
    outbox
        .claim(50, Duration::seconds(30), 10)
        .await
        .unwrap()
        .into_iter()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.event_type() == "workflow_run.failed" && envelope.subject().id == run_id
            })
        })
        .expect("the transaction committed a terminal outcome for the run")
}

async fn lifecycle_contract(db: Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let workflows = repos.workflows.clone();
    let engine = || {
        WorkflowEngine::new(
            workflows.clone(),
            repos.documents.clone(),
            repos.correspondents.clone(),
            repos.document_types.clone(),
            Arc::new(StorageFileMetadata::new(Arc::new(
                ObjectStoreDocumentStore::in_memory(),
            ))),
            "http://folio.test",
        )
        .with_ingest_access(Arc::new(IngestWorkflowAccess::new(
            repos.sources.clone(),
            repos.vaults.clone(),
            repos.members.clone(),
            &[],
            false,
        )))
    };
    let first = workflows
        .create(vault, listener("First", "document.uploaded"))
        .await
        .unwrap();
    let second = workflows
        .create(vault, listener("Second", "workflow_run.failed"))
        .await
        .unwrap();
    let root = WorkflowRun::new(
        vault,
        first.id,
        first.trigger,
        None,
        r#"{"private":"captured source data"}"#.into(),
    )
    .with_definition(&first);
    assert!(workflows.create_run(&root).await.unwrap());
    workflows
        .update(vault, first.id, listener("First", "workflow_run.failed"))
        .await
        .unwrap();
    let first_failure = claim_failure(&*repos.outbox, root.id).await;
    let first_runs = engine().handle(&first_failure).await.unwrap();
    assert_eq!(
        first_runs
            .iter()
            .map(|run| run.workflow_id)
            .collect::<Vec<_>>(),
        [second.id],
    );
    repos
        .outbox
        .mark_dispatched(first_failure.id)
        .await
        .unwrap();
    assert_eq!(
        repos
            .outbox
            .prune(Utc::now() + Duration::seconds(1))
            .await
            .unwrap(),
        1
    );

    let third = workflows
        .create(vault, listener("Third", "workflow_run.failed"))
        .await
        .unwrap();
    let mut child = workflows.find_run(first_runs[0].id).await.unwrap().unwrap();
    child.status = RunStatus::Failed;
    child.finished_at = Some(Utc::now());
    child.last_error = Some("private receiver diagnostics".into());
    workflows.update_run(&child).await.unwrap();
    let child_failure = claim_failure(&*repos.outbox, child.id).await;
    let restarted = engine();
    let (left, right) = tokio::join!(
        restarted.handle(&child_failure),
        restarted.handle(&child_failure)
    );
    let runs: Vec<_> = left.unwrap().into_iter().chain(right.unwrap()).collect();
    assert_eq!(
        runs.iter().map(|run| run.workflow_id).collect::<Vec<_>>(),
        [third.id]
    );
    assert!(runs[0].document_id.is_none());
    let payload: Value = serde_json::from_str(&runs[0].payload).unwrap();
    assert_eq!(payload["workflow_run"]["id"], json!(child.id));
    assert!(!payload.to_string().contains("private receiver diagnostics"));
}

#[tokio::test]
async fn sqlite_dispatches_committed_outcomes_after_outbox_pruning() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite:{}",
        dir.path().join("workflow-run-dispatch.db").display()
    );
    lifecycle_contract(Database::connect(&url).await.unwrap()).await;
}

#[tokio::test]
async fn postgres_dispatches_committed_outcomes_after_outbox_pruning() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow outcome dispatch",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    lifecycle_contract(Database::connect(&url).await.unwrap()).await;
}
