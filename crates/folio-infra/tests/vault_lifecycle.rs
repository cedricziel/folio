mod common;

use chrono::{Duration, Utc};
use folio_auth::User;
use folio_core::models::{
    CreateDocument, IngestionSourceInput, Principal, SourceConfig, UserId, VaultId, VaultRole,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::{EventAudience, OutboxEvent};
use folio_infra::{Database, Repositories, WebhookClient, WorkflowRunner};
use serde_json::json;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

async fn user(r: &Repositories, id: &str, admin: bool) -> UserId {
    r.auth
        .create_user(User {
            id: id.into(),
            username: id.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: admin,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    UserId::from(id)
}

async fn events(r: &Repositories, vault: VaultId) -> Vec<OutboxEvent> {
    r.outbox
        .replay(0, Utc::now() - Duration::days(1), 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.vault_id() == vault
                    && (envelope.event_type().starts_with("vault.")
                        || envelope.event_type().starts_with("vault_membership."))
            })
        })
        .collect()
}

async fn lifecycle_contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let owner = user(&r, "vault-owner", false).await;
    let viewer = user(&r, "vault-viewer", false).await;
    let admin = user(&r, "vault-admin-outsider", true).await;
    let fallback = r.vaults.instance_default().await.unwrap();
    r.members
        .grant(
            fallback,
            &Principal::User(owner.clone()),
            VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
    let lineage = MutationLineage {
        correlation_id: Uuid::new_v4(),
        causation_id: Some(Uuid::new_v4()),
    };
    let vault = with_actor(
        Some(owner.clone()),
        with_lineage(Some(lineage), r.vaults.create("Shared", Some(&owner))),
    )
    .await
    .unwrap();
    assert_eq!(
        r.access.effective_role(&owner, vault.id).await.unwrap(),
        Some(VaultRole::Owner)
    );
    let persisted_creation = r.vaults.find(vault.id).await.unwrap().unwrap();
    let metadata = serde_json::to_value(&vault).unwrap();
    assert_eq!(metadata["created_by"], json!(owner));
    assert_eq!(metadata["updated_by"], json!(owner));
    assert_eq!(metadata["created_at"], metadata["updated_at"]);
    assert_eq!(metadata["deleted_at"], json!(null));
    let initial = events(&r, vault.id).await;
    assert_eq!(initial.len(), 2);
    let created = initial[0].envelope.as_ref().unwrap();
    let granted = initial[1].envelope.as_ref().unwrap();
    assert_eq!(created.event_type(), "vault.created");
    assert_eq!(granted.event_type(), "vault_membership.granted");
    assert_eq!(created.mutation_id(), granted.mutation_id());
    assert_eq!(created.correlation_id(), lineage.correlation_id);
    assert_eq!(granted.causation_id(), lineage.causation_id);
    assert!(initial.iter().all(|event| event.event.is_none()
        && event.envelope.as_ref().unwrap().audience() == &EventAudience::Owners));

    match &db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("CREATE TRIGGER refuse_vault_event BEFORE INSERT ON outbox_events WHEN NEW.type = 'vault.updated' BEGIN SELECT RAISE(ABORT, 'refused'); END").execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("CREATE FUNCTION refuse_vault_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.type = 'vault.updated' THEN RAISE EXCEPTION 'refused'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_vault_event BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_vault_event()").execute(pool).await.unwrap();
        }
    }
    assert!(r.vaults.rename(vault.id, "Must roll back").await.is_err());
    assert_eq!(
        r.vaults.find(vault.id).await.unwrap().unwrap().name,
        "Shared"
    );
    assert_eq!(events(&r, vault.id).await.len(), 2);
    assert_eq!(r.audit.list(vault.id, 100).await.unwrap().len(), 2);
    match &db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("DROP TRIGGER refuse_vault_event")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("DROP TRIGGER refuse_vault_event ON outbox_events; DROP FUNCTION refuse_vault_event()").execute(pool).await.unwrap();
        }
    }

    let before_denied = events(&r, vault.id).await.len();
    assert!(
        with_actor(Some(viewer.clone()), r.vaults.rename(vault.id, "Denied"))
            .await
            .is_err()
    );
    assert!(
        with_actor(Some(admin.clone()), r.vaults.rename(vault.id, "Denied"))
            .await
            .is_err()
    );
    assert_eq!(events(&r, vault.id).await.len(), before_denied);

    let owner_grant = serde_json::to_value(r.members.list(vault.id).await.unwrap()).unwrap();
    r.vaults.rename(vault.id, "Shared").await.unwrap();
    r.members
        .grant(
            vault.id,
            &Principal::User(owner.clone()),
            VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
    assert_eq!(events(&r, vault.id).await.len(), 2);
    assert_eq!(
        serde_json::to_value(r.members.list(vault.id).await.unwrap()).unwrap(),
        owner_grant
    );
    with_actor(
        Some(owner.clone()),
        r.members.grant(
            vault.id,
            &Principal::User(viewer.clone()),
            VaultRole::Viewer,
            None,
        ),
    )
    .await
    .unwrap();
    with_actor(
        Some(owner.clone()),
        r.members.grant(
            vault.id,
            &Principal::User(viewer.clone()),
            VaultRole::Editor,
            None,
        ),
    )
    .await
    .unwrap();
    let changed = events(&r, vault.id).await;
    assert_eq!(
        changed
            .last()
            .unwrap()
            .envelope
            .as_ref()
            .unwrap()
            .event_type(),
        "vault_membership.role_changed"
    );
    assert_eq!(
        changed.last().unwrap().envelope.as_ref().unwrap().data()["before_role"],
        "viewer"
    );
    r.members
        .revoke(vault.id, &Principal::User(viewer.clone()))
        .await
        .unwrap();
    let count = events(&r, vault.id).await.len();
    r.members
        .revoke(vault.id, &Principal::User(viewer.clone()))
        .await
        .unwrap();
    assert_eq!(events(&r, vault.id).await.len(), count);
    assert!(r
        .members
        .revoke(vault.id, &Principal::User(owner.clone()))
        .await
        .is_err());
    assert_eq!(events(&r, vault.id).await.len(), count);
    r.members
        .grant(
            vault.id,
            &Principal::User(viewer.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();

    r.vaults.set_default(&owner, vault.id).await.unwrap();
    assert!(with_actor(Some(owner.clone()), r.vaults.delete(vault.id))
        .await
        .is_err());
    r.vaults.set_default(&owner, fallback).await.unwrap();
    r.vaults.set_default(&viewer, vault.id).await.unwrap();
    let document = r
        .documents
        .create(
            vault.id,
            CreateDocument {
                title: "Retained".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let source = r
        .sources
        .create(
            vault.id,
            IngestionSourceInput {
                name: "Retained source".into(),
                enabled: true,
                config: serde_json::from_value::<SourceConfig>(
                    json!({"type": "folder", "path": "/unused-retained-source"}),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    let workflow = r
        .workflows
        .create(
            vault.id,
            WorkflowInput {
                name: "Retained workflow".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentUpdated,
                conditions: WorkflowConditions::default(),
                actions: vec![WorkflowAction::Webhook {
                    url: "http://127.0.0.1:1/never".into(),
                    method: Default::default(),
                    headers: Default::default(),
                    secret: None,
                }],
            },
        )
        .await
        .unwrap();
    let run = WorkflowRun::new(
        vault.id,
        workflow.id,
        WorkflowTrigger::DocumentUpdated,
        Some(document.id),
        "{}".into(),
    )
    .with_actions(workflow.actions.clone());
    r.workflows.create_run(&run).await.unwrap();
    with_actor(Some(owner.clone()), r.vaults.delete(vault.id))
        .await
        .unwrap();
    assert_eq!(
        r.workflows.find_run(run.id).await.unwrap().unwrap().status,
        folio_core::models::RunStatus::Failed
    );
    let deleted_count = events(&r, vault.id).await.len();
    r.vaults.delete(vault.id).await.unwrap();
    assert_eq!(events(&r, vault.id).await.len(), deleted_count);
    assert!(r.vaults.find(vault.id).await.unwrap().is_none());
    assert!(!r
        .vaults
        .list_for_user(&owner)
        .await
        .unwrap()
        .iter()
        .any(|(entry, _)| entry.id == vault.id));
    assert_eq!(
        r.access.effective_role(&owner, vault.id).await.unwrap(),
        None
    );
    assert_eq!(
        r.access.effective_role(&viewer, vault.id).await.unwrap(),
        None
    );
    assert!(r
        .vaults
        .list_deleted_for_user(&admin)
        .await
        .unwrap()
        .is_empty());
    assert!(r
        .vaults
        .list_deleted_for_user(&viewer)
        .await
        .unwrap()
        .is_empty());
    let retained = r
        .vaults
        .list_deleted_for_user(&owner)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(retained.id, vault.id);
    assert!(serde_json::to_value(retained).unwrap()["deleted_at"].is_string());
    assert_eq!(
        r.documents.locate_retained(document.id).await.unwrap(),
        Some(vault.id)
    );
    assert!(r.sources.find(vault.id, source.id).await.unwrap().is_some());
    assert!(!r
        .sources
        .list_all()
        .await
        .unwrap()
        .iter()
        .any(|item| item.id == source.id));
    assert!(r.workflows.locate(workflow.id).await.unwrap().is_none());
    assert!(r
        .workflows
        .list_enabled(vault.id, WorkflowTrigger::DocumentUpdated)
        .await
        .unwrap()
        .is_empty());
    let runner = WorkflowRunner::new(
        r.workflows.clone(),
        r.workflow_documents.clone(),
        WebhookClient::new(),
    );
    assert_eq!(
        runner.execute(run.id).await.unwrap(),
        folio_infra::workflow::RunOutcome::Failed
    );
    assert_eq!(
        r.workflows
            .find_run(run.id)
            .await
            .unwrap()
            .unwrap()
            .completed_actions,
        0
    );
    with_actor(Some(owner.clone()), r.vaults.restore(vault.id))
        .await
        .unwrap();
    let restored_count = events(&r, vault.id).await.len();
    r.vaults.restore(vault.id).await.unwrap();
    assert_eq!(events(&r, vault.id).await.len(), restored_count);
    assert_eq!(
        r.access.effective_role(&owner, vault.id).await.unwrap(),
        Some(VaultRole::Owner)
    );
    assert!(r
        .documents
        .find_by_id(vault.id, document.id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        r.workflows.find_run(run.id).await.unwrap().unwrap().status,
        folio_core::models::RunStatus::Failed
    );
    let mut stale = run.clone();
    stale.status = folio_core::models::RunStatus::Succeeded;
    assert!(r.workflows.update_run(&stale).await.is_err());
    let restored = r.vaults.find(vault.id).await.unwrap().unwrap();
    assert_eq!(restored.created_at, persisted_creation.created_at);
    assert_eq!(restored.created_by, persisted_creation.created_by);
    assert_eq!(restored.updated_by, Some(owner));
    for event in events(&r, vault.id).await {
        r.outbox.mark_dispatched(event.id).await.unwrap();
    }
    r.outbox
        .prune(Utc::now() + Duration::days(1))
        .await
        .unwrap();
    assert!(events(&r, vault.id).await.is_empty());
    let history = r.audit.list(vault.id, 100).await.unwrap();
    assert!(history
        .iter()
        .any(|event| event.action == "vault_membership.revoked"));
    assert!(history.iter().any(|event| event.action == "vault.deleted"));
    assert!(history.iter().any(|event| event.action == "vault.restored"));

    let group = r
        .members
        .ensure_group("external", "vault-recovery")
        .await
        .unwrap();
    r.auth
        .set_external_groups(viewer.as_str(), &["vault-recovery".into()])
        .await
        .unwrap();
    let shared = r
        .vaults
        .create("Group recovery", Some(&admin))
        .await
        .unwrap();
    assert_eq!(shared.created_by.as_ref(), Some(&admin));
    assert_eq!(shared.updated_by.as_ref(), Some(&admin));
    r.members
        .grant(shared.id, &Principal::Group(group), VaultRole::Owner, None)
        .await
        .unwrap();
    r.vaults.delete(shared.id).await.unwrap();
    assert!(r
        .vaults
        .list_deleted_for_user(&viewer)
        .await
        .unwrap()
        .iter()
        .any(|v| v.id == shared.id));
    with_actor(Some(viewer.clone()), r.vaults.restore(shared.id))
        .await
        .unwrap();
    r.vaults.delete(shared.id).await.unwrap();
    r.auth
        .set_external_groups(viewer.as_str(), &[])
        .await
        .unwrap();
    assert!(!r
        .vaults
        .list_deleted_for_user(&viewer)
        .await
        .unwrap()
        .iter()
        .any(|v| v.id == shared.id));
    assert!(
        with_actor(Some(viewer.clone()), r.vaults.restore(shared.id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sqlite_retains_vaults_enforces_owners_and_publishes_transactional_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("vaults.db").display()
    ))
    .await
    .unwrap();
    lifecycle_contract(db).await;
}

#[tokio::test]
async fn postgres_retains_vaults_enforces_owners_and_publishes_transactional_lifecycle() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL vault lifecycle",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    lifecycle_contract(db).await;
}

mod scheduling {
    use super::*;
    use folio_core::models::{IngestionSourceDef, PollReport};
    use folio_core::ports::{IngestSink, IngestionSource, IngestionSourceFactory, VaultIngest};
    use folio_core::services::IngestScheduler;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::{mpsc, watch, Notify};

    struct Gate {
        started: Notify,
        release: Notify,
    }
    struct Factory {
        built: Arc<Mutex<Vec<Uuid>>>,
        polled: mpsc::UnboundedSender<(Uuid, bool)>,
        gate: Option<Arc<Gate>>,
    }
    struct Source {
        id: Uuid,
        polled: mpsc::UnboundedSender<(Uuid, bool)>,
        gate: Option<Arc<Gate>>,
    }

    impl IngestionSourceFactory for Factory {
        fn build(&self, def: &IngestionSourceDef) -> folio_core::Result<Box<dyn IngestionSource>> {
            self.built.lock().unwrap().push(def.id);
            Ok(Box::new(Source {
                id: def.id,
                polled: self.polled.clone(),
                gate: self.gate.clone(),
            }))
        }
    }
    #[async_trait::async_trait]
    impl IngestionSource for Source {
        async fn poll(&self, sink: &dyn IngestSink) -> folio_core::Result<PollReport> {
            let rejected = if let Some(gate) = &self.gate {
                gate.started.notify_one();
                gate.release.notified().await;
                sink.ingest(self.id, common::ingest::file_item(b"late source item", &[]))
                    .await
                    .is_err()
            } else {
                false
            };
            let _ = self.polled.send((self.id, rejected));
            Ok(PollReport::default())
        }
        fn interval_secs(&self) -> u64 {
            3600
        }
        async fn test(&self) -> folio_core::Result<String> {
            Ok("probe".into())
        }
    }
    fn pipeline(r: &Repositories) -> Arc<dyn VaultIngest> {
        let store: Arc<dyn folio_core::ports::DocumentStore> =
            Arc::new(folio_infra::ObjectStoreDocumentStore::in_memory());
        let importer = Arc::new(folio_core::services::Importer::new(
            r.documents.clone(),
            store.clone(),
            Arc::new(r.jobs.clone()),
        ));
        Arc::new(folio_core::services::IngestPipeline::new(
            r.documents.clone(),
            store,
            r.tags.clone(),
            r.workflows.clone(),
            r.ledger.clone(),
            importer,
        ))
    }
    fn configured(vault: VaultId, name: &str) -> IngestionSourceDef {
        serde_json::from_value(json!({ "id": Uuid::new_v4(), "name": name, "enabled": true,
            "origin": "config", "vault_id": vault, "config": { "type": "folder", "path": "/unused-probe" }
        })).unwrap()
    }
    async fn fixture() -> (Database, Repositories, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::connect(&format!(
            "sqlite:{}?mode=rwc",
            dir.path().join("scheduler.db").display()
        ))
        .await
        .unwrap();
        db.migrate().await.unwrap();
        let r = db.repositories().await;
        r.vaults.instance_default().await.unwrap();
        (db, r, dir)
    }

    #[tokio::test]
    async fn configured_sources_in_deleted_vaults_are_not_started() {
        let (_db, r, _dir) = fixture().await;
        let active = r.vaults.create("Active scheduler", None).await.unwrap().id;
        let deleted = r.vaults.create("Deleted scheduler", None).await.unwrap().id;
        r.vaults.delete(deleted).await.unwrap();
        let inactive = configured(deleted, "Inactive config");
        let sentinel = configured(active, "Active config");
        let expected = sentinel.id;
        let built = Arc::new(Mutex::new(Vec::new()));
        let (polled, mut received) = mpsc::unbounded_channel();
        let factory = Arc::new(Factory {
            built: built.clone(),
            polled,
            gate: None,
        });
        let (stop, shutdown) = watch::channel(false);
        let scheduler = IngestScheduler::new(
            r.sources.clone(),
            vec![inactive, sentinel],
            factory,
            pipeline(&r),
            r.vaults.clone(),
        );
        let running = tokio::spawn(scheduler.run(shutdown));

        let (id, _) = tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, expected);
        assert_eq!(*built.lock().unwrap(), vec![expected]);
        stop.send(true).unwrap();
        running.await.unwrap();
    }

    #[tokio::test]
    async fn a_configured_poll_cannot_import_after_its_vault_is_deleted() {
        let (_db, r, _dir) = fixture().await;
        let vault = r.vaults.create("In-flight source", None).await.unwrap().id;
        let gate = Arc::new(Gate {
            started: Notify::new(),
            release: Notify::new(),
        });
        let built = Arc::new(Mutex::new(Vec::new()));
        let (polled, mut received) = mpsc::unbounded_channel();
        let factory = Arc::new(Factory {
            built,
            polled,
            gate: Some(gate.clone()),
        });
        let sink = pipeline(&r);
        let (stop, shutdown) = watch::channel(false);
        let scheduler = IngestScheduler::new(
            r.sources.clone(),
            vec![configured(vault, "Gated config")],
            factory,
            sink.clone(),
            r.vaults.clone(),
        )
        .with_reload_interval(Duration::from_secs(3600));
        let running = tokio::spawn(scheduler.run(shutdown));
        tokio::time::timeout(Duration::from_secs(5), gate.started.notified())
            .await
            .unwrap();
        r.vaults.delete(vault).await.unwrap();

        gate.release.notify_one();
        let (_, rejected) = tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(rejected);
        assert!(r.documents.list(vault, None).await.unwrap().is_empty());
        stop.send(true).unwrap();
        running.await.unwrap();
    }
}
