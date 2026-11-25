mod common;

use chrono::{Duration, Utc};
use folio_core::models::{
    CreateDocument, DocumentDate, RunStatus, UpdateDocument, WebhookMethod, WebhookPayload,
    WorkflowAction, WorkflowCadence, WorkflowConditions, WorkflowInput, WorkflowTrigger,
};
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn configuration(cadence: WorkflowCadence) -> WorkflowInput {
    WorkflowInput {
        name: "Clock workflow".into(),
        enabled: true,
        trigger: WorkflowTrigger::Scheduled,
        triggers: Vec::new(),
        cadence: Some(cadence),
        conditions: WorkflowConditions::default(),
        actions: vec![WorkflowAction::Webhook {
            url: "https://example.test/clock".into(),
            method: WebhookMethod::Post,
            headers: Default::default(),
            secret: None,
        }],
    }
}

async fn contract(url: &str) {
    let db = Database::connect(url).await.unwrap();
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    document_date_contract(&db, url).await;
    let now = Utc::now();
    let paused_recurring = configuration(WorkflowCadence::Recurring {
        first_at: now - Duration::minutes(30),
        interval_seconds: 3600,
    });
    let paused = repos
        .workflows
        .create(
            vault,
            WorkflowInput {
                enabled: false,
                ..paused_recurring.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(now, 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    repos
        .workflows
        .update(vault, paused.id, paused_recurring)
        .await
        .unwrap();
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(now, 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(now + Duration::minutes(30), 8, "https://folio.test")
            .await
            .unwrap(),
        1
    );
    repos.workflows.delete(vault, paused.id).await.unwrap();

    let disabled = repos
        .workflows
        .create(
            vault,
            WorkflowInput {
                enabled: false,
                ..configuration(WorkflowCadence::Once {
                    at: now - Duration::minutes(30),
                })
            },
        )
        .await
        .unwrap();
    repos
        .workflows
        .update(
            vault,
            disabled.id,
            configuration(WorkflowCadence::Recurring {
                first_at: now - Duration::minutes(90),
                interval_seconds: 3600,
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(now, 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(now + Duration::minutes(30), 8, "https://folio.test")
            .await
            .unwrap(),
        1
    );
    repos.workflows.delete(vault, disabled.id).await.unwrap();

    let at = now + Duration::hours(1);
    let once = configuration(WorkflowCadence::Once { at });
    let workflow = repos.workflows.create(vault, once.clone()).await.unwrap();
    assert_eq!(workflow.cadence, once.cadence);
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(at - Duration::nanoseconds(1), 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );

    let pending_before = repos.jobs.pending_count().await.unwrap();
    let restarted = Database::connect(url).await.unwrap().repositories().await;
    let (left, right) = tokio::join!(
        repos
            .workflows
            .enqueue_due_schedules(at, 8, "https://folio.test"),
        restarted
            .workflows
            .enqueue_due_schedules(at, 8, "https://folio.test"),
    );
    assert_eq!(left.unwrap() + right.unwrap(), 1);
    assert_eq!(
        restarted
            .workflows
            .enqueue_due_schedules(at, 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    let runs = restarted
        .workflows
        .list_runs(vault, workflow.id, 10)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    let payload: WebhookPayload = serde_json::from_str(&run.payload).unwrap();
    assert_eq!(run.definition_revision, Some(workflow.revision));
    assert_eq!(run.actions, once.actions);
    assert_eq!(payload.event, WorkflowTrigger::Scheduled);
    assert_eq!(payload.occurred_at, at);
    assert!(run.document_id.is_none() && payload.document.is_none());
    assert_eq!(
        restarted.jobs.pending_count().await.unwrap(),
        pending_before + 1
    );

    let later = configuration(WorkflowCadence::Once {
        at: at + Duration::hours(1),
    });
    repos
        .workflows
        .update(vault, workflow.id, later)
        .await
        .unwrap();
    repos
        .workflows
        .update(vault, workflow.id, once)
        .await
        .unwrap();
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(at, 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        repos
            .workflows
            .find_run(run.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Failed
    );

    let cadence = WorkflowCadence::Recurring {
        first_at: at,
        interval_seconds: 3600,
    };
    let recurring = configuration(cadence);
    let clock = repos
        .workflows
        .create(vault, recurring.clone())
        .await
        .unwrap();
    let catch_up = at + Duration::hours(100);
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(catch_up, 8, "https://folio.test")
            .await
            .unwrap(),
        16
    );
    let runs = repos
        .workflows
        .list_runs(vault, clock.id, 32)
        .await
        .unwrap();
    let mut instants: Vec<_> = runs
        .iter()
        .map(|run| {
            serde_json::from_str::<WebhookPayload>(&run.payload)
                .unwrap()
                .occurred_at
        })
        .collect();
    instants.sort();
    assert_eq!(instants.len(), 16);
    assert_eq!(instants.first(), Some(&(at + Duration::hours(85))));
    assert_eq!(instants.last(), Some(&catch_up));

    let changed = WorkflowInput {
        name: "Updated clock".into(),
        ..recurring
    };
    let paused = WorkflowInput {
        enabled: false,
        ..changed.clone()
    };
    repos
        .workflows
        .update(vault, clock.id, paused)
        .await
        .unwrap();
    let next = catch_up + Duration::hours(1);
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(next, 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    let resumed = repos
        .workflows
        .update(vault, clock.id, changed)
        .await
        .unwrap();
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(next, 8, "https://folio.test")
            .await
            .unwrap(),
        1
    );
    let resumed_runs = repos
        .workflows
        .list_runs(vault, clock.id, 32)
        .await
        .unwrap();
    assert_eq!(
        resumed_runs
            .iter()
            .filter(|run| run.status == RunStatus::Pending)
            .count(),
        1
    );
    let newest = resumed_runs
        .iter()
        .find(|run| run.status == RunStatus::Pending)
        .unwrap();
    assert_eq!(newest.definition_revision, Some(resumed.revision));

    let selections = vec![
        WorkflowTrigger::DocumentRestored,
        WorkflowTrigger::MetadataFieldUpdated,
    ];
    let converted = repos
        .workflows
        .replace(
            vault,
            clock.id,
            WorkflowInput {
                name: resumed.name,
                enabled: resumed.enabled,
                trigger: selections[0],
                triggers: selections.clone(),
                cadence: None,
                conditions: resumed.conditions,
                actions: resumed.actions,
            },
            Some(resumed.revision),
        )
        .await
        .unwrap();
    assert_eq!(converted.triggers, selections);
    assert!(converted.cadence.is_none());
    for trigger in selections {
        let selected = repos.workflows.list_enabled(vault, trigger).await.unwrap();
        assert!(selected.iter().any(|workflow| workflow.id == clock.id));
    }
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(next + Duration::hours(1), 8, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    let cancelled = repos.workflows.find_run(newest.id).await.unwrap().unwrap();
    assert_eq!(cancelled.status, RunStatus::Failed);
    assert_eq!(cancelled.definition_revision, Some(resumed.revision));
    assert_eq!(cancelled.payload, newest.payload);

    let failing_at = next + Duration::minutes(10);
    let failing = repos
        .workflows
        .create(
            vault,
            configuration(WorkflowCadence::Once { at: failing_at }),
        )
        .await
        .unwrap();
    match &db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("CREATE TRIGGER deny_clock_job BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;")
                .execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("CREATE FUNCTION deny_clock_job() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture failure'; END; $$; CREATE TRIGGER deny_clock_job BEFORE INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION deny_clock_job();")
                .execute(pool).await.unwrap();
        }
    }
    assert!(repos
        .workflows
        .enqueue_due_schedules(failing_at, 8, "https://folio.test")
        .await
        .is_err());
    assert!(repos
        .workflows
        .list_runs(vault, failing.id, 10)
        .await
        .unwrap()
        .is_empty());
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query("DROP TRIGGER deny_clock_job")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DROP TRIGGER deny_clock_job ON jobs")
                .execute(pool)
                .await
                .unwrap();
        }
    }
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(failing_at, 8, "https://folio.test")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repos
            .workflows
            .list_runs(vault, failing.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

async fn document_date_contract(db: &Database, url: &str) {
    let repos = db.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let other = repos.vaults.create("Other dates", None).await.unwrap().id;
    let time = "09:30:00".parse().unwrap();
    let at = (Utc::now() + Duration::days(2))
        .date_naive()
        .and_time(time)
        .and_utc();
    let cadence = WorkflowCadence::DocumentDateOffset {
        days: -1,
        time_utc: time,
    };
    let date = cadence.document_date_at(at).unwrap();
    let create = |name: String| CreateDocument {
        title: name.clone(),
        original_filename: Some(name),
        ..Default::default()
    };
    let set_date = |date: DocumentDate| UpdateDocument {
        document_date: Some(Some(date)),
        ..Default::default()
    };
    let mut ids = Vec::with_capacity(65);
    for index in 0..65 {
        let document = repos
            .documents
            .create(vault, create(format!("invoice-{index}.pdf")))
            .await
            .unwrap();
        repos
            .documents
            .update(vault, document.id, set_date(date))
            .await
            .unwrap();
        ids.push(document.id);
    }
    for (owner, filename) in [
        (other, "invoice-other.pdf"),
        (vault, "receipt.pdf"),
        (vault, "invoice-trash.pdf"),
    ] {
        let document = repos
            .documents
            .create(owner, create(filename.into()))
            .await
            .unwrap();
        repos
            .documents
            .update(owner, document.id, set_date(date))
            .await
            .unwrap();
        if filename == "invoice-trash.pdf" {
            repos.documents.delete(owner, document.id).await.unwrap();
        }
    }
    repos
        .documents
        .create(vault, create("invoice-undated.pdf".into()))
        .await
        .unwrap();

    let workflow = repos
        .workflows
        .create(
            vault,
            WorkflowInput {
                name: "Relative to invoice date".into(),
                trigger: WorkflowTrigger::DocumentDateOffset,
                conditions: WorkflowConditions {
                    filename_glob: Some("invoice-*.pdf".into()),
                    ..Default::default()
                },
                ..configuration(cadence)
            },
        )
        .await
        .unwrap();

    let shortly_before = at - Duration::nanoseconds(1);
    for _ in 0..3 {
        assert_eq!(
            repos
                .workflows
                .enqueue_due_schedules(shortly_before, 4, "https://folio.test")
                .await
                .unwrap(),
            0
        );
    }
    let restarted = Database::connect(url).await.unwrap().repositories().await;
    let (left, right) = tokio::join!(
        repos
            .workflows
            .enqueue_due_schedules(at, 4, "https://folio.test"),
        restarted
            .workflows
            .enqueue_due_schedules(at, 4, "https://folio.test"),
    );
    let initially_queued = left.unwrap() + right.unwrap();
    assert!((63..=65).contains(&initially_queued));
    let remaining = restarted
        .workflows
        .enqueue_due_schedules(at, 4, "https://folio.test")
        .await
        .unwrap();
    assert_eq!(initially_queued + remaining, 65);
    assert_eq!(
        restarted
            .workflows
            .enqueue_due_schedules(at, 4, "https://folio.test")
            .await
            .unwrap(),
        0
    );

    let runs = repos
        .workflows
        .list_runs(vault, workflow.id, 100)
        .await
        .unwrap();
    assert_eq!(runs.len(), 65);
    let mut received: Vec<_> = runs
        .iter()
        .map(|run| {
            let document = run.document_id.unwrap();
            let payload: WebhookPayload = serde_json::from_str(&run.payload).unwrap();
            assert_eq!(run.event, WorkflowTrigger::DocumentDateOffset);
            assert_eq!(run.definition_revision, Some(workflow.revision));
            assert_eq!(payload.occurred_at, at);
            let snapshot = payload.document.unwrap();
            assert_eq!(snapshot.id, document);
            assert_eq!(snapshot.document_date, Some(date));
            assert_eq!(
                snapshot.urls.api,
                format!("https://folio.test/api/documents/{document}")
            );
            document
        })
        .collect();
    received.sort_unstable();
    ids.sort_unstable();
    assert_eq!(received, ids);

    let next_at = at + Duration::days(1);
    let next_date = cadence.document_date_at(next_at).unwrap();
    repos
        .documents
        .update(vault, ids[0], set_date(next_date))
        .await
        .unwrap();
    assert_eq!(
        restarted
            .workflows
            .enqueue_due_schedules(next_at, 4, "https://folio.test")
            .await
            .unwrap(),
        1
    );
    let runs = repos
        .workflows
        .list_runs(vault, workflow.id, 100)
        .await
        .unwrap();
    let received: Vec<_> = runs
        .iter()
        .filter(|run| run.document_id == Some(ids[0]))
        .map(|run| {
            serde_json::from_str::<WebhookPayload>(&run.payload)
                .unwrap()
                .document
                .unwrap()
                .document_date
                .unwrap()
        })
        .collect();
    assert_eq!(received.len(), 2);
    assert!(received.contains(&date));
    assert!(received.contains(&next_date));

    let selected = vec![
        WorkflowTrigger::DocumentRestored,
        WorkflowTrigger::DocumentUpdated,
    ];
    let converted = repos
        .workflows
        .replace(
            vault,
            workflow.id,
            WorkflowInput {
                trigger: selected[0],
                triggers: selected.clone(),
                cadence: None,
                name: workflow.name,
                enabled: workflow.enabled,
                conditions: workflow.conditions,
                actions: workflow.actions,
            },
            Some(workflow.revision),
        )
        .await
        .unwrap();
    assert_eq!(converted.triggers, selected);
    assert!(converted.cadence.is_none());
    assert_eq!(
        restarted
            .workflows
            .enqueue_due_schedules(next_at + Duration::days(1), 4, "https://folio.test")
            .await
            .unwrap(),
        0
    );
    let retained = repos
        .workflows
        .list_runs(vault, workflow.id, 100)
        .await
        .unwrap();
    assert_eq!(retained.len(), runs.len());
    for run in retained {
        let original = runs.iter().find(|old| old.id == run.id).unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.payload, original.payload);
        assert_eq!(run.definition_revision, Some(workflow.revision));
    }
    repos.workflows.delete(vault, workflow.id).await.unwrap();
}

#[tokio::test]
async fn sqlite_clock_claims_are_durable() {
    let dir = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite:{}",
        dir.path().join("workflow-schedule.db").display()
    ))
    .await;
}

#[tokio::test]
async fn postgres_clock_claims_are_durable() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow schedules",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    contract(&url).await;
}
