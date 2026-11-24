mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use folio_core::domain_event::{EventAudience, DOCUMENT_METADATA_CHANGED};
use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Id, MetadataChange,
    MetadataFieldInput, MetadataKind, MetadataValue, ReviewOrigin, RunStatus, UpdateDocument,
    UserId, VaultId, WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun,
    WorkflowTrigger,
};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::services::WorkflowEngine;
use folio_core::EventOrigin;
use folio_infra::database::Repositories;
use folio_infra::{
    workflow::{RunOutcome, MAX_ATTEMPTS},
    Database, ObjectStoreDocumentStore, StorageFileMetadata, WebhookClient, WorkflowRunner,
};
use serde_json::json;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn queue(
    workflows: &dyn folio_core::ports::WorkflowRepository,
    vault: VaultId,
    document: Id,
    actions: Vec<WorkflowAction>,
) -> WorkflowRun {
    let workflow = workflows
        .create(
            vault,
            WorkflowInput {
                cadence: None,
                name: "File invoice".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentUploaded,
                triggers: Vec::new(),
                conditions: WorkflowConditions::default(),
                actions,
            },
        )
        .await
        .unwrap();
    let run = WorkflowRun::new(
        vault,
        workflow.id,
        workflow.trigger,
        Some(document),
        "{}".into(),
    )
    .with_definition(&workflow);
    assert!(workflows.create_run(&run).await.unwrap());
    run
}

async fn reject_progress(db: &Database) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("CREATE TRIGGER reject_workflow_progress BEFORE UPDATE OF completed_actions ON workflow_runs WHEN NEW.completed_actions > OLD.completed_actions BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;")
                .execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("CREATE FUNCTION reject_workflow_progress() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.completed_actions > OLD.completed_actions THEN RAISE EXCEPTION 'fixture failure'; END IF; RETURN NEW; END; $$; CREATE TRIGGER reject_workflow_progress BEFORE UPDATE OF completed_actions ON workflow_runs FOR EACH ROW EXECUTE FUNCTION reject_workflow_progress();")
                .execute(pool).await.unwrap();
        }
    }
}

async fn allow_progress(db: &Database) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DROP TRIGGER reject_workflow_progress")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("DROP TRIGGER reject_workflow_progress ON workflow_runs; DROP FUNCTION reject_workflow_progress();")
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let document = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Invoice".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let correspondent = repos
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Customer".into(),
            },
        )
        .await
        .unwrap();
    let run = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![WorkflowAction::SetCorrespondent {
            correspondent_id: correspondent.id,
        }],
    )
    .await;
    let runner = WorkflowRunner::new(
        repos.workflows.clone(),
        repos.workflow_documents.clone(),
        WebhookClient::new(),
    );

    let since = Utc::now() - Duration::minutes(1);
    let before = repos.outbox.replay(0, since, 100).await.unwrap().len();
    reject_progress(&db).await;
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Retry);
    let uncommitted = repos
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(uncommitted.correspondent_id, None);
    assert_eq!(
        repos.outbox.replay(0, since, 100).await.unwrap().len(),
        before
    );
    assert_eq!(
        repos
            .workflows
            .find_run(run.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        0
    );

    allow_progress(&db).await;
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    let applied = repos
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(applied.correspondent_id, Some(correspondent.id));
    assert_eq!(
        repos
            .workflows
            .find_run(run.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        1
    );

    let tag = repos
        .tags
        .create(
            vault,
            CreateTag {
                name: "Invoice".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let kind = repos
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Bill".into(),
            },
        )
        .await
        .unwrap();
    repos
        .documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                document_type_id: Some(Some(kind.id)),
                tag_ids: Some(vec![tag.id]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ordered = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![
            WorkflowAction::RenameDocument {
                title: "Automated title".into(),
            },
            WorkflowAction::RemoveTags {
                tag_ids: vec![tag.id],
            },
            WorkflowAction::ClearCorrespondent,
            WorkflowAction::ClearDocumentType,
            WorkflowAction::SetDocumentDate {
                date: "2026-11-20".parse().unwrap(),
            },
            WorkflowAction::ClearDocumentDate,
            WorkflowAction::SetReviewed { reviewed: true },
            WorkflowAction::TrashDocument,
        ],
    )
    .await;
    let (first, second) = tokio::join!(
        repos
            .workflow_documents
            .apply_workflow_document_action(&ordered, 0),
        repos
            .workflow_documents
            .apply_workflow_document_action(&ordered, 0),
    );
    assert!(!first.unwrap());
    assert!(!second.unwrap());
    let committed = repos.workflows.find_run(ordered.id).await.unwrap().unwrap();
    assert_eq!(committed.completed_actions, 1);
    assert_eq!(committed.status, RunStatus::Pending);
    let mutated = repos
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mutated.title, "Automated title");
    assert!(matches!(
        repos.workflows.update_run(&ordered).await,
        Err(folio_core::Error::Conflict(_))
    ));

    repos
        .documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("Manual title".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runner.execute(ordered.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    let trashed = repos
        .documents
        .find_trashed_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trashed.title, "Manual title");
    assert_eq!(trashed.correspondent_id, None);
    assert_eq!(trashed.document_type_id, None);
    assert_eq!(trashed.document_date, None);
    assert_eq!(trashed.review_origin, Some(ReviewOrigin::System));
    assert_eq!(trashed.reviewed_by, None);
    assert_eq!(
        runner.execute(ordered.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    let events = repos.outbox.replay(0, since, 100).await.unwrap();
    let workflow_events = |name| {
        events
            .iter()
            .filter(|event| {
                event.origin == folio_core::EventOrigin::Workflow
                    && event
                        .envelope
                        .as_ref()
                        .is_some_and(|envelope| envelope.event_type() == name)
            })
            .count()
    };
    assert_eq!(workflow_events("document.title_changed"), 1);
    assert_eq!(workflow_events("document.date_changed"), 2);

    let outcomes: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| {
            event.event_type() == "workflow_run.succeeded" && event.subject().id == ordered.id
        })
        .collect();
    let [outcome] = outcomes.as_slice() else {
        panic!("Expected one success publication for the completed ordered run");
    };
    assert_eq!(outcome.audience(), &EventAudience::Owners);
    assert_eq!(outcome.vault_id(), vault);
    assert_eq!(outcome.document_id(), None);
    assert_eq!(
        outcome.data(),
        &serde_json::json!({"workflow_id": ordered.workflow_id})
    );

    assert_eq!(
        repos
            .workflows
            .find_run(ordered.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        8
    );
    repos.documents.restore(vault, document.id).await.unwrap();
    assert!(repos
        .documents
        .get_tags(vault, document.id)
        .await
        .unwrap()
        .is_empty());

    let mut final_step = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![WorkflowAction::RenameDocument {
            title: "Final title".into(),
        }],
    )
    .await;
    final_step.attempts = MAX_ATTEMPTS;
    final_step.last_error = Some("Previous attempt failed".into());
    repos.workflows.update_run(&final_step).await.unwrap();
    assert!(repos
        .workflow_documents
        .apply_workflow_document_action(&final_step, 0)
        .await
        .unwrap());

    let persisted = repos
        .workflows
        .find_run(final_step.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.completed_actions, 1);
    assert_eq!(persisted.attempts, final_step.attempts);
    assert_eq!(persisted.status, RunStatus::Succeeded);
    assert!(persisted.finished_at.is_some());
    assert_eq!(persisted.last_error, None);
    let renamed = repos
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title, "Final title");
    assert_eq!(
        runner.execute(final_step.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    assert!(matches!(
        repos
            .workflow_documents
            .apply_workflow_document_action(&final_step, 0)
            .await,
        Err(folio_core::Error::Conflict(_))
    ));
    let events = repos.outbox.replay(0, since, 100).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.origin == folio_core::EventOrigin::Workflow
                && event
                    .envelope
                    .as_ref()
                    .is_some_and(|envelope| envelope.event_type() == "workflow_run.succeeded"
                        && envelope.subject().id == final_step.id
                        && envelope.audience() == &EventAudience::Owners))
            .count(),
        1
    );

    let mut progressed = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![WorkflowAction::RenameDocument {
            title: "Already superseded".into(),
        }],
    )
    .await;
    progressed.completed_actions = 1;
    repos.workflows.update_run(&progressed).await.unwrap();
    assert!(!repos
        .workflow_documents
        .apply_workflow_document_action(&progressed, 0)
        .await
        .unwrap());
    assert_eq!(
        runner.execute(progressed.id).await.unwrap(),
        RunOutcome::Succeeded
    );

    let queued_rename = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![WorkflowAction::RenameDocument {
            title: "Queued title".into(),
        }],
    )
    .await;
    let invalid_actions = serde_json::to_string(&[WorkflowAction::RenameDocument {
        title: "Invoice\0copy".into(),
    }])
    .unwrap();
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE workflow_runs SET actions = ? WHERE id = ?")
                .bind(&invalid_actions)
                .bind(queued_rename.id.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE workflow_runs SET actions = $1 WHERE id = $2")
                .bind(&invalid_actions)
                .bind(queued_rename.id)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    assert_eq!(
        runner.execute(queued_rename.id).await.unwrap(),
        RunOutcome::Failed
    );
    let rejected = repos
        .workflows
        .find_run(queued_rename.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rejected.last_error.as_deref(),
        Some("Validation error: Document title cannot contain a NUL character")
    );
    assert_eq!(rejected.completed_actions, 0);
    let unchanged = repos
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.title, "Final title");

    let foreign_vault = repos.vaults.create("Other vault", None).await.unwrap();
    let foreign_tag = repos
        .tags
        .create(
            foreign_vault.id,
            CreateTag {
                name: "Other tag".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let invalid = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![WorkflowAction::AddTags {
            tag_ids: vec![tag.id, foreign_tag.id],
        }],
    )
    .await;
    assert_eq!(
        runner.execute(invalid.id).await.unwrap(),
        RunOutcome::Failed
    );
    assert!(repos
        .documents
        .get_tags(vault, document.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        repos
            .workflows
            .find_run(invalid.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        0
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let interrupted = queue(
        repos.workflows.as_ref(),
        vault,
        document.id,
        vec![
            WorkflowAction::Webhook {
                url,
                method: Default::default(),
                headers: Default::default(),
                secret: None,
            },
            WorkflowAction::RenameDocument {
                title: "Resumed after the saved webhook".into(),
            },
        ],
    )
    .await;
    let mut newer = interrupted.clone();
    newer.attempts = 2;
    newer.completed_actions = 1;
    let workflows = repos.workflows.clone();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move || {
                let (workflows, newer) = (workflows.clone(), newer.clone());
                async move {
                    workflows.update_run(&newer).await.unwrap();
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let outcome = runner.execute(interrupted.id).await.unwrap();
    server.abort();
    assert_eq!(outcome, RunOutcome::Retry);
    let retained = repos
        .workflows
        .find_run(interrupted.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.status, RunStatus::Pending);
    assert_eq!(retained.completed_actions, 1);
    assert_eq!(retained.attempts, 2);
    assert_eq!(
        runner.execute(interrupted.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    assert_eq!(
        repos
            .documents
            .find_by_id(vault, document.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "Resumed after the saved webhook"
    );
    metadata_actions(&db, &repos, vault, document.id).await;
}

async fn metadata_actions(db: &Database, repos: &Repositories, vault: VaultId, document: Id) {
    let field = repos
        .metadata_fields
        .create(
            vault,
            MetadataFieldInput {
                name: "Invoice number".into(),
                kind: MetadataKind::Integer,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    let run = queue(
        repos.workflows.as_ref(),
        vault,
        document,
        serde_json::from_value(json!([
            {"type": "set_metadata", "field_id": field.id, "value": {"kind": "integer", "value": "0042"}},
            {"type": "clear_metadata", "field_id": field.id}
        ]))
        .unwrap(),
    )
    .await;
    let before = repos.metadata_values.get(vault, document).await.unwrap();
    let since = Utc::now() - Duration::minutes(1);
    let count = repos.outbox.replay(0, since, 100).await.unwrap().len();

    reject_progress(db).await;
    assert!(repos
        .workflow_documents
        .apply_workflow_document_action(&run, 0)
        .await
        .is_err());
    let rolled_back = repos.metadata_values.get(vault, document).await.unwrap();
    assert_eq!(before.revision, rolled_back.revision);
    assert_eq!(before.values, rolled_back.values);
    assert_eq!(
        repos.outbox.replay(0, since, 100).await.unwrap().len(),
        count
    );
    assert_eq!(
        repos
            .workflows
            .find_run(run.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        0
    );
    allow_progress(db).await;

    let lineage = MutationLineage {
        correlation_id: Id::new_v4(),
        causation_id: Some(Id::new_v4()),
    };
    with_actor(
        Some(UserId::from("unrelated-user")),
        with_lineage(
            Some(lineage),
            repos
                .workflow_documents
                .apply_workflow_document_action(&run, 0),
        ),
    )
    .await
    .unwrap();
    let applied = repos.metadata_values.get(vault, document).await.unwrap();
    assert_eq!(
        applied.values[0].value,
        Some(MetadataValue::Integer("42".into()))
    );

    assert_eq!(applied.values[0].updated_by, None);

    let manual = repos
        .metadata_values
        .patch(
            vault,
            document,
            applied.revision,
            vec![MetadataChange {
                field_id: field.id,
                value: Some(MetadataValue::Integer("18".into())),
            }],
        )
        .await
        .unwrap();
    let reopened = db.repositories().await;
    reopened
        .workflow_documents
        .apply_workflow_document_action(&run, 0)
        .await
        .unwrap();
    let after_replay = reopened.metadata_values.get(vault, document).await.unwrap();
    assert_eq!(after_replay.revision, manual.revision);
    assert_eq!(after_replay.values, manual.values);

    let runner = WorkflowRunner::new(
        reopened.workflows.clone(),
        reopened.workflow_documents.clone(),
        WebhookClient::new(),
    );
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    let cleared = reopened.metadata_values.get(vault, document).await.unwrap();
    assert_eq!(cleared.values.len(), 1);
    assert_eq!(cleared.values[0].value, None);
    assert_eq!(
        reopened
            .workflows
            .find_run(run.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        2
    );

    let deliveries: Vec<_> = reopened
        .outbox
        .replay(0, since, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|envelope| envelope.event_type() == DOCUMENT_METADATA_CHANGED)
        })
        .collect();
    let events: Vec<_> = deliveries
        .iter()
        .filter(|event| event.origin == EventOrigin::Workflow)
        .map(|event| event.envelope.as_ref().unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].correlation_id(), lineage.correlation_id);
    assert_eq!(events[0].causation_id(), lineage.causation_id);
    assert!(events.iter().all(|event| event.actor_id().is_none()));
    assert_eq!(
        events[0].data()["changes"][0]["after"],
        json!({"kind": "integer", "value": "42"})
    );
    assert_eq!(
        events[1].data()["changes"][0]["before"],
        json!({"kind": "integer", "value": "18"})
    );
    assert!(events[1].data()["changes"][0]["after"].is_null());

    reopened
        .workflows
        .create(
            vault,
            WorkflowInput {
                name: "Watch field edits".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentMetadataChanged,
                triggers: Vec::new(),
                cadence: None,
                conditions: WorkflowConditions::default(),
                actions: vec![WorkflowAction::AddTags {
                    tag_ids: vec![Id::new_v4()],
                }],
            },
        )
        .await
        .unwrap();
    let engine = WorkflowEngine::new(
        reopened.workflows.clone(),
        reopened.documents.clone(),
        reopened.correspondents.clone(),
        reopened.document_types.clone(),
        Arc::new(StorageFileMetadata::new(Arc::new(
            ObjectStoreDocumentStore::in_memory(),
        ))),
        "http://folio.test",
    );
    assert_eq!(deliveries.len(), 3);
    for delivery in deliveries {
        let runs = engine.handle(&delivery).await.unwrap();
        assert_eq!(
            runs.len(),
            usize::from(delivery.origin == EventOrigin::User)
        );
    }

    let foreign_vault = repos.vaults.create("Field owner", None).await.unwrap().id;
    let foreign = repos
        .metadata_fields
        .create(
            foreign_vault,
            MetadataFieldInput {
                name: "Private number".into(),
                kind: MetadataKind::Integer,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    let denied = queue(
        repos.workflows.as_ref(),
        vault,
        document,
        serde_json::from_value(json!([
            {"type": "set_metadata", "field_id": foreign.id, "value": {"kind": "integer", "value": "1"}}
        ])).unwrap(),
    ).await;
    assert_eq!(runner.execute(denied.id).await.unwrap(), RunOutcome::Failed);
    let after_denied = reopened.metadata_values.get(vault, document).await.unwrap();
    assert_eq!(after_denied.revision, cleared.revision);
    assert_eq!(after_denied.values, cleared.values);
}

#[tokio::test]
async fn sqlite_workflow_progress_and_document_mutation_commit_together() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite:{}",
        dir.path().join("workflow-actions.db").display()
    );
    contract(Database::connect(&url).await.unwrap()).await;
}

#[tokio::test]
async fn postgres_workflow_progress_and_document_mutation_commit_together() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow actions",
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
