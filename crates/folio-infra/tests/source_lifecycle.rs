mod common;

use chrono::{Duration, Utc};
use folio_core::models::{IngestionSourceInput, LedgerEntry, SourceStatus, UserId};
use folio_core::mutation::with_actor;
use folio_core::ports::OutboxStore;
use folio_infra::{Database, PostgresOutboxStore, SqliteOutboxStore};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

fn input(name: &str) -> IngestionSourceInput {
    IngestionSourceInput {
        name: name.into(),
        enabled: true,
        config: serde_json::from_value(
            serde_json::json!({"type":"folder", "path":"/unused/test-folder"}),
        )
        .unwrap(),
    }
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let source = with_actor(
        Some(UserId::from("alice")),
        r.sources.create(vault, input("Inbox")),
    )
    .await
    .unwrap();
    let original = source.lifecycle.clone().unwrap();
    assert_eq!(original.created_by, Some(UserId::from("alice")));
    assert_eq!(original.updated_by, original.created_by);
    let unchanged = with_actor(
        Some(UserId::from("ignored")),
        r.sources.update(vault, source.id, input("Inbox")),
    )
    .await
    .unwrap();
    assert_eq!(unchanged, source);
    let changed = with_actor(
        Some(UserId::from("bob")),
        r.sources.update(vault, source.id, input("Renamed Inbox")),
    )
    .await
    .unwrap();
    assert_eq!(
        changed.lifecycle.as_ref().unwrap().created_at,
        original.created_at
    );
    assert_eq!(
        changed.lifecycle.as_ref().unwrap().updated_by,
        Some(UserId::from("bob"))
    );
    let ledger = LedgerEntry {
        vault_id: vault,
        key: "retained".into(),
        source_id: source.id,
        document_id: None,
        outcome: "retryable".into(),
        created_at: chrono::DateTime::from_timestamp_micros(Utc::now().timestamp_micros()).unwrap(),
    };
    assert!(r.ledger.insert(vault, &ledger).await.unwrap());
    let status = SourceStatus {
        last_imported: 4,
        ..Default::default()
    };
    r.sources.save_status(source.id, &status).await.unwrap();
    with_actor(
        Some(UserId::from("carol")),
        r.sources.delete(vault, source.id),
    )
    .await
    .unwrap();
    assert!(r.sources.find(vault, source.id).await.unwrap().is_none());
    assert!(r.sources.list_all().await.unwrap().is_empty());
    let retained = r.ledger.find(vault, "retained").await.unwrap().unwrap();
    assert_eq!(retained.source_id, ledger.source_id);
    assert_eq!(retained.outcome, ledger.outcome);
    assert_eq!(retained.created_at, ledger.created_at);
    let deleted = r
        .sources
        .find_deleted(vault, source.id)
        .await
        .unwrap()
        .unwrap();
    let deleted_metadata = deleted.lifecycle.as_ref().unwrap();
    assert!(deleted_metadata.deleted_at.is_some());
    assert_eq!(deleted_metadata.updated_by, Some(UserId::from("carol")));
    r.sources.delete(vault, source.id).await.unwrap();
    assert_eq!(
        r.sources
            .find_deleted(vault, source.id)
            .await
            .unwrap()
            .unwrap(),
        deleted
    );
    assert!(r
        .sources
        .update(vault, source.id, input("deleted update"))
        .await
        .is_err());
    assert!(r
        .sources
        .find_deleted(folio_core::models::VaultId::new(), source.id)
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        r.ledger
            .insert(
                vault,
                &LedgerEntry {
                    key: "blocked".into(),
                    ..ledger.clone()
                }
            )
            .await,
        Err(folio_core::Error::Conflict(_))
    ));
    assert!(r
        .ledger
        .transition_if_matches(vault, &ledger, "claimed", Utc::now())
        .await
        .unwrap());
    let configured = LedgerEntry {
        key: "configured-exception".into(),
        source_id: uuid::Uuid::new_v4(),
        ..ledger.clone()
    };
    assert!(r.ledger.insert(vault, &configured).await.unwrap());
    let replacement = r
        .sources
        .create(vault, input("Renamed Inbox"))
        .await
        .unwrap();
    assert!(matches!(
        r.sources.restore(vault, source.id).await,
        Err(folio_core::Error::Duplicate { .. })
    ));
    assert_eq!(
        r.sources
            .find_deleted(vault, source.id)
            .await
            .unwrap()
            .unwrap(),
        deleted
    );
    r.sources.delete(vault, replacement.id).await.unwrap();
    let restored = with_actor(
        Some(UserId::from("dan")),
        r.sources.restore(vault, source.id),
    )
    .await
    .unwrap();
    assert_eq!(restored.id, source.id);
    assert_eq!(
        restored.lifecycle.as_ref().unwrap().created_by,
        original.created_by
    );
    assert_eq!(
        restored.lifecycle.as_ref().unwrap().created_at,
        original.created_at
    );
    assert_eq!(
        restored.lifecycle.as_ref().unwrap().updated_by,
        Some(UserId::from("dan"))
    );
    assert!(restored.lifecycle.as_ref().unwrap().deleted_at.is_none());
    assert_eq!(r.sources.restore(vault, source.id).await.unwrap(), restored);
    assert_eq!(r.sources.status(source.id).await.unwrap(), Some(status));
    assert_eq!(r.sources.list_all().await.unwrap(), vec![restored.clone()]);
    let outbox: Box<dyn OutboxStore> = match &db {
        Database::Sqlite(pool) => Box::new(SqliteOutboxStore::new(pool.clone())),
        Database::Postgres(pool) => Box::new(PostgresOutboxStore::new(pool.clone())),
    };
    let events = outbox.claim(20, Duration::seconds(30), 10).await.unwrap();
    let envelopes: Vec<_> = events
        .iter()
        .map(|e| e.envelope.as_ref().unwrap())
        .collect();
    let types: Vec<_> = envelopes.iter().map(|e| e.event_type()).collect();
    assert_eq!(
        types,
        vec![
            "source.created",
            "source.updated",
            "source.deleted",
            "source.created",
            "source.deleted",
            "source.restored"
        ]
    );
    assert_eq!(envelopes[0].actor_id(), Some(&UserId::from("alice")));
    assert_eq!(envelopes[2].actor_id(), Some(&UserId::from("carol")));
    assert_eq!(envelopes[5].actor_id(), Some(&UserId::from("dan")));
    assert_eq!(
        envelopes[0].audience(),
        &folio_core::domain_event::EventAudience::Owners
    );
    assert!(envelopes[0].document_id().is_none());
    assert_eq!(
        envelopes[0].data(),
        &serde_json::json!({"name":"Inbox", "kind":"folder", "enabled":true})
    );
    let rollback_restore = r
        .sources
        .create(vault, input("Rollback restore"))
        .await
        .unwrap();
    r.sources.delete(vault, rollback_restore.id).await.unwrap();
    reject_events(&db).await;
    assert!(r.sources.create(vault, input("Rejected")).await.is_err());
    assert_eq!(r.sources.list(vault).await.unwrap(), vec![restored.clone()]);
    assert!(r
        .sources
        .update(vault, source.id, input("Rejected update"))
        .await
        .is_err());
    assert!(r.sources.delete(vault, source.id).await.is_err());
    assert!(r.sources.restore(vault, rollback_restore.id).await.is_err());
    assert_eq!(
        r.sources.find(vault, source.id).await.unwrap(),
        Some(restored)
    );
    assert!(r
        .sources
        .find_deleted(vault, rollback_restore.id)
        .await
        .unwrap()
        .is_some());
}

async fn reject_events(db: &Database) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql("CREATE TRIGGER reject_source_events BEFORE INSERT ON outbox_events WHEN NEW.type LIKE 'source.%' BEGIN SELECT RAISE(ABORT, 'rejected source event'); END;").execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql("CREATE FUNCTION reject_source_events() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.type LIKE 'source.%' THEN RAISE EXCEPTION 'rejected source event'; END IF; RETURN NEW; END; $$; CREATE TRIGGER reject_source_events BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION reject_source_events();").execute(pool).await.unwrap();
        }
    }
}

#[tokio::test]
async fn sqlite_source_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    contract(
        Database::connect(&format!(
            "sqlite:{}",
            dir.path().join("sources.db").display()
        ))
        .await
        .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn postgres_source_lifecycle() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL source lifecycle",
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
                .filter(|m| m.version < 20261102200000)
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
    let created = chrono::DateTime::parse_from_rfc3339("2026-11-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let config = serde_json::to_string(&input("Legacy source").config).unwrap();
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query("INSERT INTO ingestion_sources (id,vault_id,name,enabled,kind,config,created_at,updated_at) VALUES (?,?,'Legacy source',1,'folder',?,?,?)")
                .bind(id.to_string()).bind(vault.to_string()).bind(&config).bind(created.to_rfc3339()).bind(created.to_rfc3339()).execute(pool).await.unwrap();
            sqlx::query("INSERT INTO ingest_ledger (vault_id,key,source_id,outcome,created_at) VALUES (?,'legacy-ledger',?,'imported',?)")
                .bind(vault.to_string()).bind(id.to_string()).bind(created.to_rfc3339()).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("INSERT INTO ingestion_sources (id,vault_id,name,enabled,kind,config,created_at,updated_at) VALUES ($1,$2,'Legacy source',true,'folder',$3,$4,$4)")
                .bind(id).bind(vault.0).bind(&config).bind(created).execute(pool).await.unwrap();
            sqlx::query("INSERT INTO ingest_ledger (vault_id,key,source_id,outcome,created_at) VALUES ($1,'legacy-ledger',$2,'imported',$3)")
                .bind(vault.0).bind(id).bind(created).execute(pool).await.unwrap();
        }
    }
    let status = SourceStatus {
        last_imported: 7,
        ..Default::default()
    };
    r.sources.save_status(id, &status).await.unwrap();
    db.migrate().await.unwrap();
    let retained = r.sources.find(vault, id).await.unwrap().unwrap();
    let lifecycle = retained.lifecycle.unwrap();
    assert_eq!(lifecycle.created_at, created);
    assert_eq!(lifecycle.updated_at, created);
    assert_eq!(lifecycle.created_by, None);
    assert_eq!(lifecycle.updated_by, None);
    assert_eq!(lifecycle.deleted_at, None);
    assert_eq!(r.sources.status(id).await.unwrap(), Some(status));
    assert_eq!(
        r.ledger
            .find(vault, "legacy-ledger")
            .await
            .unwrap()
            .unwrap()
            .source_id,
        id
    );
}

#[tokio::test]
async fn sqlite_upgrade_preserves_source_metadata_status_and_ledger() {
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
async fn postgres_upgrade_preserves_source_metadata_status_and_ledger() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL source lifecycle upgrade",
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
