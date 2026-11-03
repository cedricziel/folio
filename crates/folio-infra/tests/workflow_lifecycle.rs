mod common;

use chrono::{Duration, Utc};
use folio_core::domain_event::EventAudience;
use folio_core::models::{
    RunStatus, UserId, WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun,
    WorkflowTrigger,
};
use folio_core::mutation::with_actor;
use folio_core::ports::OutboxStore;
use folio_infra::{Database, PostgresOutboxStore, SqliteOutboxStore};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

fn input(name: &str) -> WorkflowInput {
    WorkflowInput {
        name: name.into(),
        enabled: true,
        trigger: WorkflowTrigger::DocumentUploaded,
        conditions: WorkflowConditions::default(),
        actions: vec![WorkflowAction::Webhook {
            url: "http://127.0.0.1:1/unused".into(),
            method: folio_core::models::WebhookMethod::Post,
            headers: Default::default(),
            secret: Some("retained private value".into()),
        }],
    }
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let workflow = with_actor(
        Some(UserId::from("alice")),
        r.workflows.create(vault, input("Retained workflow")),
    )
    .await
    .unwrap();
    assert_eq!(workflow.created_by, Some(UserId::from("alice")));
    assert_eq!(workflow.updated_by, workflow.created_by);
    let unchanged = with_actor(
        Some(UserId::from("ignored")),
        r.workflows
            .update(vault, workflow.id, input("Retained workflow")),
    )
    .await
    .unwrap();
    assert_eq!(unchanged.updated_at, workflow.updated_at);
    assert_eq!(unchanged.updated_by, workflow.updated_by);
    let changed = with_actor(
        Some(UserId::from("bob")),
        r.workflows
            .update(vault, workflow.id, input("Changed workflow")),
    )
    .await
    .unwrap();
    assert_eq!(changed.created_at, workflow.created_at);
    assert_eq!(changed.updated_by, Some(UserId::from("bob")));
    let run = WorkflowRun::new(
        vault,
        workflow.id,
        workflow.trigger,
        Some(uuid::Uuid::new_v4()),
        r#"{"note":"retained event facts"}"#.into(),
    )
    .with_actions(workflow.actions.clone());
    assert!(r.workflows.create_run(&run).await.unwrap());
    let mut succeeded = WorkflowRun::new(
        vault,
        workflow.id,
        workflow.trigger,
        Some(uuid::Uuid::new_v4()),
        r#"{"note":"successful event"}"#.into(),
    )
    .with_actions(workflow.actions.clone());
    r.workflows.create_run(&succeeded).await.unwrap();
    succeeded.status = RunStatus::Succeeded;
    succeeded.finished_at = Some(Utc::now());
    r.workflows.update_run(&succeeded).await.unwrap();
    with_actor(
        Some(UserId::from("carol")),
        r.workflows.delete(vault, workflow.id),
    )
    .await
    .unwrap();
    assert!(r
        .workflows
        .find_by_id(vault, workflow.id)
        .await
        .unwrap()
        .is_none());
    assert!(r.workflows.locate(workflow.id).await.unwrap().is_none());
    assert!(r
        .workflows
        .list_enabled(vault, workflow.trigger)
        .await
        .unwrap()
        .is_empty());
    assert!(r.workflows.list(vault).await.unwrap().is_empty());
    let deleted = r
        .workflows
        .find_deleted(vault, workflow.id)
        .await
        .unwrap()
        .unwrap();
    assert!(deleted.deleted_at.is_some());
    assert_eq!(deleted.updated_by, Some(UserId::from("carol")));
    let retained = r
        .workflows
        .find_run(run.id)
        .await
        .unwrap()
        .expect("deletion retains execution history");
    assert_eq!(retained.status, RunStatus::Failed);
    assert!(retained.finished_at.is_some());
    assert_eq!(retained.actions, run.actions);
    assert_eq!(retained.payload, run.payload);
    assert_eq!(
        r.workflows
            .find_run(succeeded.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Succeeded
    );
    assert_eq!(
        r.workflows
            .list_runs(vault, workflow.id, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    let rejected = WorkflowRun::new(
        vault,
        workflow.id,
        workflow.trigger,
        None,
        r#"{"note":"stale event"}"#.into(),
    );
    assert!(r.workflows.create_run(&rejected).await.is_err());
    r.workflows.delete(vault, workflow.id).await.unwrap();
    let repeated_delete = r
        .workflows
        .find_deleted(vault, workflow.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repeated_delete.updated_at, deleted.updated_at);
    assert_eq!(repeated_delete.updated_by, deleted.updated_by);
    assert!(r
        .workflows
        .update(vault, workflow.id, input("Deleted edit"))
        .await
        .is_err());
    assert!(r
        .workflows
        .restore(folio_core::models::VaultId::new(), workflow.id)
        .await
        .is_err());
    let restored = with_actor(
        Some(UserId::from("dan")),
        r.workflows.restore(vault, workflow.id),
    )
    .await
    .unwrap();
    assert_eq!(restored.id, workflow.id);
    assert_eq!(restored.name, changed.name);
    assert_eq!(restored.created_at, workflow.created_at);
    assert_eq!(restored.created_by, workflow.created_by);
    assert_eq!(restored.updated_by, Some(UserId::from("dan")));
    assert!(restored.deleted_at.is_none());
    let unchanged = r.workflows.restore(vault, workflow.id).await.unwrap();
    assert_eq!(unchanged.updated_at, restored.updated_at);
    assert_eq!(unchanged.updated_by, restored.updated_by);
    assert_eq!(r.workflows.locate(workflow.id).await.unwrap(), Some(vault));
    assert_eq!(
        r.workflows
            .list_enabled(vault, workflow.trigger)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(!r.workflows.create_run(&run).await.unwrap());
    assert!(matches!(
        r.workflows.update_run(&run).await,
        Err(folio_core::Error::Conflict(_))
    ));
    let mut stale_success = run.clone();
    stale_success.status = RunStatus::Succeeded;
    assert!(matches!(
        r.workflows.update_run(&stale_success).await,
        Err(folio_core::Error::Conflict(_))
    ));
    assert_eq!(
        r.workflows.find_run(run.id).await.unwrap().unwrap().status,
        RunStatus::Failed
    );
    let future = WorkflowRun::new(
        vault,
        workflow.id,
        workflow.trigger,
        Some(uuid::Uuid::new_v4()),
        r#"{"note":"future event"}"#.into(),
    )
    .with_actions(restored.actions.clone());
    assert!(r.workflows.create_run(&future).await.unwrap());
    let outbox: Box<dyn OutboxStore> = match &db {
        Database::Sqlite(pool) => Box::new(SqliteOutboxStore::new(pool.clone())),
        Database::Postgres(pool) => Box::new(PostgresOutboxStore::new(pool.clone())),
    };
    let claimed = outbox.claim(50, Duration::seconds(30), 10).await.unwrap();
    let events: Vec<_> = claimed
        .iter()
        .filter_map(|e| e.envelope.as_ref())
        .filter(|e| e.event_type().starts_with("workflow."))
        .collect();
    assert_eq!(
        events.iter().map(|e| e.event_type()).collect::<Vec<_>>(),
        vec![
            "workflow.created",
            "workflow.updated",
            "workflow.deleted",
            "workflow.restored"
        ]
    );
    assert_eq!(events[0].actor_id(), Some(&UserId::from("alice")));
    assert_eq!(events[2].actor_id(), Some(&UserId::from("carol")));
    assert_eq!(events[3].actor_id(), Some(&UserId::from("dan")));
    assert_eq!(events[0].audience(), &EventAudience::Owners);
    assert_eq!(
        events[0].data(),
        &serde_json::json!({"name":"Retained workflow", "enabled":true, "trigger":"document.uploaded"})
    );
    let rollback_restore = r
        .workflows
        .create(vault, input("Rollback restore"))
        .await
        .unwrap();
    r.workflows
        .delete(vault, rollback_restore.id)
        .await
        .unwrap();
    reject_events(&db).await;
    assert!(r.workflows.create(vault, input("Rejected")).await.is_err());
    assert!(r
        .workflows
        .update(vault, workflow.id, input("Rejected edit"))
        .await
        .is_err());
    assert!(r.workflows.delete(vault, workflow.id).await.is_err());
    assert!(r
        .workflows
        .restore(vault, rollback_restore.id)
        .await
        .is_err());
    let after = r
        .workflows
        .find_by_id(vault, workflow.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.name, restored.name);
    assert_eq!(after.updated_at, restored.updated_at);
    assert_eq!(
        r.workflows
            .find_run(future.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Pending
    );
    assert!(r
        .workflows
        .find_deleted(vault, rollback_restore.id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(r.workflows.list(vault).await.unwrap().len(), 1);
}

async fn reject_events(db: &Database) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("CREATE TRIGGER reject_workflow_events BEFORE INSERT ON outbox_events WHEN NEW.type LIKE 'workflow.%' BEGIN SELECT RAISE(ABORT, 'rejected workflow event'); END;").execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("CREATE FUNCTION reject_workflow_events() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.type LIKE 'workflow.%' THEN RAISE EXCEPTION 'rejected workflow event'; END IF; RETURN NEW; END; $$; CREATE TRIGGER reject_workflow_events BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION reject_workflow_events();").execute(pool).await.unwrap();
        }
    }
}

#[tokio::test]
async fn sqlite_workflow_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    contract(
        Database::connect(&format!(
            "sqlite:{}",
            dir.path().join("workflow.db").display()
        ))
        .await
        .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn postgres_workflow_lifecycle() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow lifecycle",
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

async fn upgrade_contract(db: Database) {
    use sqlx::migrate::Migrator;
    use std::borrow::Cow;
    let all = match &db {
        Database::Sqlite(_) => sqlx::migrate!("../../migrations/sqlite"),
        Database::Postgres(_) => sqlx::migrate!("../../migrations/postgres"),
    };
    let old = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|m| m.version < 20261102200300)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    match &db {
        Database::Sqlite(pool) => old.run(pool).await.unwrap(),
        Database::Postgres(pool) => old.run(pool).await.unwrap(),
    }
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let id = uuid::Uuid::new_v4();
    let run_id = uuid::Uuid::new_v4();
    let created = chrono::DateTime::parse_from_rfc3339("2026-11-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let actions = serde_json::to_string(&input("Legacy").actions).unwrap();
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query("INSERT INTO workflows (id,vault_id,name,enabled,trigger_event,conditions,actions,created_at,updated_at) VALUES (?,?,'Legacy',1,'document.uploaded','{}',?,?,?)")
                .bind(id.to_string()).bind(vault.to_string()).bind(&actions).bind(created.to_rfc3339()).bind(created.to_rfc3339()).execute(pool).await.unwrap();
            sqlx::query("INSERT INTO workflow_runs (id,workflow_id,event,status,payload,created_at) VALUES (?,?,'document.uploaded','pending','retained payload',?)")
                .bind(run_id.to_string()).bind(id.to_string()).bind(created.to_rfc3339()).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("INSERT INTO workflows (id,vault_id,name,enabled,trigger_event,conditions,actions,created_at,updated_at) VALUES ($1,$2,'Legacy',true,'document.uploaded','{}',$3,$4,$4)")
                .bind(id).bind(vault.0).bind(&actions).bind(created).execute(pool).await.unwrap();
            sqlx::query("INSERT INTO workflow_runs (id,workflow_id,event,status,payload,created_at) VALUES ($1,$2,'document.uploaded','pending','retained payload',$3)")
                .bind(run_id).bind(id).bind(created).execute(pool).await.unwrap();
        }
    }
    db.migrate().await.unwrap();
    let retained = r.workflows.find_by_id(vault, id).await.unwrap().unwrap();
    assert_eq!(retained.created_at, created);
    assert_eq!(retained.updated_at, created);
    assert_eq!(retained.created_by, None);
    assert_eq!(retained.updated_by, None);
    assert_eq!(retained.deleted_at, None);
    assert_eq!(retained.actions, input("Legacy").actions);
    let unchanged = r
        .workflows
        .update(vault, id, input("Legacy"))
        .await
        .unwrap();
    assert_eq!(unchanged.updated_at, created);
    let edited = with_actor(
        Some(UserId::from("legacy-editor")),
        r.workflows.update(vault, id, input("Legacy renamed")),
    )
    .await
    .unwrap();
    assert_eq!(edited.created_at, created);
    assert_ne!(edited.updated_at, created);
    assert_eq!(edited.updated_by, Some(UserId::from("legacy-editor")));
    let run = r.workflows.find_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Pending);
    assert_eq!(run.payload, "retained payload");
}

#[tokio::test]
async fn sqlite_workflow_upgrade_retains_configuration_and_runs() {
    let dir = tempfile::tempdir().unwrap();
    upgrade_contract(
        Database::connect(&format!(
            "sqlite:{}",
            dir.path().join("upgrade.db").display()
        ))
        .await
        .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn postgres_workflow_upgrade_retains_configuration_and_runs() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL workflow lifecycle upgrade",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    upgrade_contract(Database::connect(&url).await.unwrap()).await;
}
