mod common;

use chrono::{Duration, Utc};
use folio_core::models::{
    RunStatus, WebhookMethod, WebhookPayload, WorkflowAction, WorkflowCadence, WorkflowConditions,
    WorkflowInput, WorkflowTrigger,
};
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn configuration(cadence: WorkflowCadence) -> WorkflowInput {
    WorkflowInput {
        name: "Clock workflow".into(),
        enabled: true,
        trigger: WorkflowTrigger::Scheduled,
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
        repos.workflows.enqueue_due_schedules(now, 8).await.unwrap(),
        0
    );
    repos
        .workflows
        .update(vault, paused.id, paused_recurring)
        .await
        .unwrap();
    assert_eq!(
        repos.workflows.enqueue_due_schedules(now, 8).await.unwrap(),
        0
    );
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(now + Duration::minutes(30), 8)
            .await
            .unwrap(),
        1
    );
    repos.workflows.delete(vault, paused.id).await.unwrap();

    let at = now + Duration::hours(1);
    let once = configuration(WorkflowCadence::Once { at });
    let workflow = repos.workflows.create(vault, once.clone()).await.unwrap();
    assert_eq!(workflow.cadence, once.cadence);
    assert_eq!(
        repos
            .workflows
            .enqueue_due_schedules(at - Duration::nanoseconds(1), 8)
            .await
            .unwrap(),
        0
    );

    let pending_before = repos.jobs.pending_count().await.unwrap();
    let restarted = Database::connect(url).await.unwrap().repositories().await;
    let (left, right) = tokio::join!(
        repos.workflows.enqueue_due_schedules(at, 8),
        restarted.workflows.enqueue_due_schedules(at, 8),
    );
    assert_eq!(left.unwrap() + right.unwrap(), 1);
    assert_eq!(
        restarted
            .workflows
            .enqueue_due_schedules(at, 8)
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
        repos.workflows.enqueue_due_schedules(at, 8).await.unwrap(),
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
            .enqueue_due_schedules(catch_up, 8)
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
            .enqueue_due_schedules(next, 8)
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
            .enqueue_due_schedules(next, 8)
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
        .enqueue_due_schedules(failing_at, 8)
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
            .enqueue_due_schedules(failing_at, 8)
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
