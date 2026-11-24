use folio_core::models::{
    FolderAfterImport, FolderConfig, IngestClaim, IngestItemKey, IngestResult,
    IngestionSourceInput, SourceConfig, SourceKind, SourceOrigin, SourceStatus,
};
use folio_infra::Database;

async fn scope_contract(database: Database) {
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let other = repos.vaults.create("Other", None).await.unwrap().id;
    let input = IngestionSourceInput {
        name: "scope contract".into(),
        enabled: true,
        config: SourceConfig::Folder(FolderConfig {
            path: "/unused-test-source".into(),
            recursive: false,
            subfolders_as_tags: false,
            after_import: FolderAfterImport::Delete,
            settle_secs: 0,
            rescan_secs: 60,
        }),
    };
    workflow_outcome_contract(&repos, input.clone()).await;
    let source = repos.sources.create(vault, input.clone()).await.unwrap();
    let key = IngestItemKey::folder(b"private.pdf", b"stage", b"source scope");
    let fingerprint = "d".repeat(64);
    assert!(matches!(
        repos
            .ingest_outcomes
            .claim(
                other,
                source.id,
                SourceKind::Folder,
                SourceOrigin::Managed,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Busy
    ));
    let IngestClaim::Claimed(attempt) = repos
        .ingest_outcomes
        .claim(
            vault,
            source.id,
            SourceKind::Folder,
            SourceOrigin::Managed,
            &key,
            &fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("active managed source must claim")
    };
    repos
        .sources
        .update(
            vault,
            source.id,
            IngestionSourceInput {
                enabled: false,
                ..input.clone()
            },
        )
        .await
        .unwrap();
    assert!(repos
        .ingest_outcomes
        .complete(&attempt, IngestResult::Skipped, None, None)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .ingest_outcomes
        .page(vault, source.id, None, 10)
        .await
        .unwrap()
        .outcomes
        .is_empty());
    repos.sources.update(vault, source.id, input).await.unwrap();
    let pending_key = IngestItemKey::folder(b"pending.pdf", b"stage", b"trash during poll");
    let IngestClaim::Claimed(pending) = repos
        .ingest_outcomes
        .claim(
            vault,
            source.id,
            SourceKind::Folder,
            SourceOrigin::Managed,
            &pending_key,
            &fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("enabled source must claim before trash")
    };
    repos.sources.delete(vault, source.id).await.unwrap();
    assert!(matches!(
        repos
            .ingest_outcomes
            .claim(
                vault,
                source.id,
                SourceKind::Folder,
                SourceOrigin::Managed,
                &IngestItemKey::folder(b"new.pdf", b"stage", b"observed after trash"),
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Busy
    ));
    assert!(
        repos
            .ingest_outcomes
            .complete(&pending, IngestResult::Skipped, None, None)
            .await
            .unwrap()
            .is_none(),
        "a poll already in progress must not publish after trash"
    );
    let failure = SourceStatus {
        last_error: Some("test failure".into()),
        ..Default::default()
    };
    assert!(repos
        .ingest_outcomes
        .record_source_status(
            vault,
            source.id,
            SourceOrigin::Managed,
            SourceKind::Folder,
            &failure
        )
        .await
        .is_err());
    assert!(repos.sources.status(source.id).await.unwrap().is_none());
    let configured = uuid::Uuid::new_v4();
    assert!(matches!(
        repos
            .ingest_outcomes
            .claim(
                vault,
                configured,
                SourceKind::Folder,
                SourceOrigin::Config,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Claimed(_)
    ));
}

async fn workflow_outcome_contract(repos: &folio_infra::Repositories, input: IngestionSourceInput) {
    use folio_core::models::{WebhookMethod, WorkflowAction, WorkflowInput, WorkflowTrigger};
    use folio_core::services::{IngestWorkflowAccess, WorkflowEngine};
    use folio_infra::workflow::{RunOutcome, WorkflowRunner};
    use folio_infra::{ObjectStoreDocumentStore, StorageFileMetadata, WebhookClient};
    use std::sync::Arc;

    let owner = "workflow-outcome-owner";
    repos
        .auth
        .create_user(folio_auth::User {
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
    let vault = repos
        .vaults
        .create("Workflow outcomes", Some(&owner.into()))
        .await
        .unwrap()
        .id;
    let source = repos.sources.create(vault, input).await.unwrap();
    let access = Arc::new(IngestWorkflowAccess::new(
        repos.sources.clone(),
        repos.vaults.clone(),
        repos.members.clone(),
        &[],
        true,
    ));
    let engine = WorkflowEngine::new(
        repos.workflows.clone(),
        repos.documents.clone(),
        repos.correspondents.clone(),
        repos.document_types.clone(),
        Arc::new(StorageFileMetadata::new(Arc::new(
            ObjectStoreDocumentStore::in_memory(),
        ))),
        "http://folio.test",
    )
    .with_ingest_access(access.clone());
    let runner = WorkflowRunner::new(
        repos.workflows.clone(),
        repos.workflow_documents.clone(),
        WebhookClient::new(),
    )
    .with_ingest_access(access);
    let definition = WorkflowInput {
        cadence: None,
        name: "Failed item delivery".into(),
        enabled: true,
        trigger: WorkflowTrigger::IngestFailed,
        conditions: folio_core::models::WorkflowConditions {
            event: Some(folio_core::models::WorkflowEventConditions {
                ingest: Some(folio_core::models::WorkflowIngestConditions {
                    source_id: Some(source.id),
                    outcome: Some(IngestResult::Failed),
                    retryable: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        actions: vec![WorkflowAction::Webhook {
            url: "http://127.0.0.1:1/hook".into(),
            method: WebhookMethod::Post,
            headers: Default::default(),
            secret: None,
        }],
    };
    let workflow = repos
        .workflows
        .create(vault, definition.clone())
        .await
        .unwrap();
    let key = IngestItemKey::folder(b"private-filename.pdf", b"stage", b"test content");
    let fingerprint = "e".repeat(64);
    let mut attempts = Vec::new();
    for _ in 0..2 {
        let IngestClaim::Claimed(attempt) = repos
            .ingest_outcomes
            .claim(
                vault,
                source.id,
                SourceKind::Folder,
                SourceOrigin::Managed,
                &key,
                &fingerprint,
            )
            .await
            .unwrap()
        else {
            panic!("expected an attempted import")
        };
        repos
            .ingest_outcomes
            .complete(&attempt, IngestResult::Failed, None, None)
            .await
            .unwrap()
            .expect("completed outcome");
        attempts.push(attempt);
    }
    assert_eq!(attempts[0].item_id, attempts[1].item_id);
    assert_ne!(attempts[0].attempt_id, attempts[1].attempt_id);
    let events = repos
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap();
    let mut runs = Vec::new();
    for attempt in attempts {
        let event = events
            .iter()
            .find(|event| {
                event
                    .envelope
                    .as_ref()
                    .is_some_and(|envelope| envelope.mutation_id() == attempt.attempt_id)
            })
            .unwrap();
        let matched = engine.handle(event).await.unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].workflow_id, workflow.id);
        assert!(matched[0].document_id.is_none());
        assert!(!matched[0].payload.contains("private-filename"));
        assert!(engine.handle(event).await.unwrap().is_empty());
        runs.extend(matched);
    }

    let health_workflow = repos
        .workflows
        .create(
            vault,
            WorkflowInput {
                trigger: WorkflowTrigger::SourceFailed,
                conditions: Default::default(),
                ..definition.clone()
            },
        )
        .await
        .unwrap();
    repos
        .ingest_outcomes
        .record_source_status(
            vault,
            source.id,
            SourceOrigin::Managed,
            SourceKind::Folder,
            &SourceStatus {
                last_error: Some("private-provider-error".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let health_events = repos
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap();
    let failed_health = health_events
        .iter()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.event_type() == "source.failed" && envelope.subject().id == source.id
            })
        })
        .unwrap();
    let health_runs = engine.handle(failed_health).await.unwrap();
    assert_eq!(health_runs.len(), 1);
    assert_eq!(health_runs[0].workflow_id, health_workflow.id);
    assert!(!health_runs[0].payload.contains("private-provider-error"));

    repos.sources.delete(vault, source.id).await.unwrap();
    assert_eq!(
        runner.execute(runs[0].id).await.unwrap(),
        RunOutcome::Failed
    );
    repos
        .workflows
        .create(
            vault,
            WorkflowInput {
                trigger: WorkflowTrigger::SourceFailed,
                conditions: Default::default(),
                ..definition
            },
        )
        .await
        .unwrap();
    assert!(engine.handle(failed_health).await.unwrap().is_empty());
    let run = repos.workflows.find_run(runs[0].id).await.unwrap().unwrap();
    assert_eq!(
        run.last_error.as_deref(),
        Some("Source or workflow is no longer authorized")
    );
}

#[tokio::test]
async fn sqlite_managed_source_must_still_be_active_in_its_vault() {
    scope_contract(Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_managed_source_must_still_be_active_in_its_vault() {
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let container = match started {
        Ok(container) => container,
        Err(error) => {
            assert!(
                !std::env::var("FOLIO_REQUIRE_DOCKER").is_ok_and(|v| !v.is_empty() && v != "0"),
                "PostgreSQL required: {error}"
            );
            eprintln!("SKIPPED: PostgreSQL unavailable: {error}");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    scope_contract(
        Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}
