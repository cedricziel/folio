mod common;

use chrono::{Duration, Utc};
use folio_core::domain_event::EventAudience;
use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Id, ReviewOrigin,
    RunStatus, UpdateDocument, VaultId, WorkflowAction, WorkflowConditions, WorkflowInput,
    WorkflowRun, WorkflowTrigger,
};
use folio_infra::{
    workflow::{RunOutcome, MAX_ATTEMPTS},
    Database, WebhookClient, WorkflowRunner,
};
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
    match &db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("CREATE TRIGGER reject_workflow_progress BEFORE UPDATE OF completed_actions ON workflow_runs WHEN NEW.completed_actions > OLD.completed_actions BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;")
                .execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("CREATE FUNCTION reject_workflow_progress() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.completed_actions > OLD.completed_actions THEN RAISE EXCEPTION 'fixture failure'; END IF; RETURN NEW; END; $$; CREATE TRIGGER reject_workflow_progress BEFORE UPDATE OF completed_actions ON workflow_runs FOR EACH ROW EXECUTE FUNCTION reject_workflow_progress();")
                .execute(pool).await.unwrap();
        }
    }
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

    match &db {
        Database::Sqlite(pool) => {
            sqlx::query("DROP TRIGGER reject_workflow_progress")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DROP TRIGGER reject_workflow_progress ON workflow_runs")
                .execute(pool)
                .await
                .unwrap();
        }
    }
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
