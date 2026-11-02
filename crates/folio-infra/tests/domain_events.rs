mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::{CreateDocument, CreateTag, UpdateDocument, UpdateTag, VaultId};
use folio_core::ports::OutboxStore;
use folio_core::{DeletedDocument, DocumentEvent, EventOrigin};
use folio_infra::database::{postgres, sqlite};
use folio_infra::{Database, PostgresOutboxStore, SqliteOutboxStore};
use serde_json::json;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

fn envelope() -> DomainEventEnvelope {
    DomainEventEnvelope::new(
        "tag.created",
        VaultId::new(),
        DomainEventSubject {
            subject_type: "tag".into(),
            id: Uuid::new_v4(),
        },
        None,
        EventAudience::Vault,
        json!({"name": "history"}),
    )
    .unwrap()
}

async fn insert(
    db: &Database,
    event: &DomainEventEnvelope,
    commit: bool,
) -> folio_core::Result<i64> {
    match db {
        Database::Sqlite(pool) => {
            let mut tx = pool.begin().await.unwrap();
            let id = sqlite::insert_domain_event(&mut tx, event).await?;
            if commit {
                tx.commit().await.unwrap();
            } else {
                tx.rollback().await.unwrap();
            }
            Ok(id)
        }
        Database::Postgres(pool) => {
            let mut tx = pool.begin().await.unwrap();
            let id = postgres::insert_domain_event(&mut tx, event).await?;
            if commit {
                tx.commit().await.unwrap();
            } else {
                tx.rollback().await.unwrap();
            }
            Ok(id)
        }
    }
}

async fn domain_outbox_contract(db: Database) {
    db.migrate().await.unwrap();
    let outbox: Box<dyn OutboxStore> = match &db {
        Database::Sqlite(pool) => Box::new(SqliteOutboxStore::new(pool.clone())),
        Database::Postgres(pool) => Box::new(PostgresOutboxStore::new(pool.clone())),
    };
    let rolled_back = envelope();
    insert(&db, &rolled_back, false).await.unwrap();
    assert!(outbox
        .claim(10, Duration::seconds(30), 10)
        .await
        .unwrap()
        .is_empty());

    let event = envelope();
    let (first, duplicate) = tokio::join!(insert(&db, &event, true), insert(&db, &event, true));
    let id = first.unwrap();
    assert_eq!(duplicate.unwrap(), id);
    let claimed = outbox.claim(10, Duration::seconds(30), 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert!(claimed[0].event.is_none());
    assert_eq!(claimed[0].envelope.as_ref(), Some(&event));

    outbox
        .release(id, "retry", Utc::now() - Duration::seconds(1))
        .await
        .unwrap();
    let retried = outbox.claim(10, Duration::seconds(30), 10).await.unwrap();
    assert_eq!(retried[0].id, id);
    assert_eq!(retried[0].envelope.as_ref(), Some(&event));
    assert_eq!(retried[0].attempts, 2);

    let mut different = serde_json::to_value(&event).unwrap();
    different["data"] = json!({"name": "rewritten"});
    let different: DomainEventEnvelope = serde_json::from_value(different).unwrap();
    assert!(matches!(
        insert(&db, &different, true).await,
        Err(folio_core::Error::Conflict(_))
    ));

    explicit_deletion_snapshot_survives_new_live_metadata(&db).await;
}

async fn explicit_deletion_snapshot_survives_new_live_metadata(db: &Database) {
    let r = db.repositories().await;
    let vault = r
        .vaults
        .create("Historical snapshot", None)
        .await
        .unwrap()
        .id;
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "Historical tag".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Historical title".into(),
                original_filename: Some("historical.pdf".into()),
                mime_type: Some("application/pdf".into()),
                tag_ids: vec![tag.id],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let snapshot = Arc::new(DeletedDocument {
        document: document.clone(),
        tags: vec![tag.clone()],
    });
    r.tags
        .update(
            vault,
            tag.id,
            UpdateTag {
                name: Some("Current tag".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("Current title".into()),
                tag_ids: Some(Vec::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let id = r
        .outbox
        .append(
            &DocumentEvent::Deleted {
                document_id: document.id,
                snapshot: Some(snapshot),
            },
            EventOrigin::User,
        )
        .await
        .unwrap();
    let stored = r
        .outbox
        .replay(0, Utc::now() - Duration::days(1), 100)
        .await
        .unwrap()
        .into_iter()
        .find(|event| event.id == id)
        .unwrap();

    let context = stored.context.as_ref().unwrap();
    assert_eq!(context.title, "Historical title");
    assert_eq!(context.original_filename.as_deref(), Some("historical.pdf"));
    assert_eq!(context.tags.len(), 1);
    assert_eq!(context.tags[0].id, tag.id);
    assert_eq!(context.tags[0].name, "Historical tag");
    let envelope = stored.envelope.as_ref().unwrap();
    assert_eq!(envelope.data()["title"], "Historical title");
    assert_eq!(
        envelope.data()["tags"],
        json!([{"id": tag.id, "name": "Historical tag"}])
    );
}

#[tokio::test]
async fn sqlite_domain_events_are_transactional_idempotent_and_stable_across_retries() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("events.db").display()
    ))
    .await
    .unwrap();
    domain_outbox_contract(db).await;
}

#[tokio::test]
async fn postgres_domain_events_are_transactional_idempotent_and_stable_across_retries() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL domain event contract",
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
    domain_outbox_contract(db).await;
}
