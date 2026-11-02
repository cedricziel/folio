mod common;

use folio_core::models::{MetadataFieldInput, MetadataKind, UserId, VaultId};
use folio_core::ports::MetadataFieldRepository;
use folio_infra::database::{
    postgres::PostgresMetadataFieldRepository, sqlite::SqliteMetadataFieldRepository,
};
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn input(name: &str) -> MetadataFieldInput {
    MetadataFieldInput {
        name: name.into(),
        kind: MetadataKind::Text,
        required: false,
        unique: false,
    }
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    let vault = repositories.vaults.instance_default().await.unwrap();
    let repo: Box<dyn MetadataFieldRepository> = match &db {
        Database::Sqlite(pool) => Box::new(SqliteMetadataFieldRepository::new(pool.clone())),
        Database::Postgres(pool) => Box::new(PostgresMetadataFieldRepository::new(pool.clone())),
    };
    let actor = UserId::from("metadata-test-editor");
    let field = folio_core::mutation::with_actor(
        Some(actor.clone()),
        repo.create(vault, input(" Reference ")),
    )
    .await
    .unwrap();
    assert_eq!(field.name, "Reference");
    assert_eq!(field.created_by, Some(actor));
    assert_eq!(field.created_at, field.updated_at);
    assert_eq!(field.vault_id, vault);
    assert!(repo.create(vault, input("REFERENCE")).await.is_err());
    assert!(repo.find(VaultId::new(), field.id).await.unwrap().is_none());
    assert!(repo
        .update(VaultId::new(), field.id, input("Invisible"))
        .await
        .is_err());
    let unchanged = repo
        .update(vault, field.id, input("Reference"))
        .await
        .unwrap();
    assert_eq!(unchanged.updated_at, field.updated_at);
    let renamed = repo
        .update(vault, field.id, input("Invoice reference"))
        .await
        .unwrap();
    assert_eq!(renamed.id, field.id);
    assert_eq!(renamed.created_at, field.created_at);
    assert_eq!(renamed.created_by, field.created_by);
    assert_eq!(renamed.updated_by, None);
    let retired = repo.set_deleted(vault, field.id, true).await.unwrap();
    assert!(retired.deleted_at.is_some());
    let repeated = repo.set_deleted(vault, field.id, true).await.unwrap();
    assert_eq!(repeated.updated_at, retired.updated_at);
    assert!(repo.list(vault, false, None, 10).await.unwrap().is_empty());
    assert_eq!(repo.list(vault, true, None, 10).await.unwrap().len(), 1);
    let replacement = repo
        .create(vault, input("Invoice reference"))
        .await
        .unwrap();
    assert!(repo.set_deleted(vault, field.id, false).await.is_err());
    repo.set_deleted(vault, replacement.id, true).await.unwrap();
    let restored = repo.set_deleted(vault, field.id, false).await.unwrap();
    assert_eq!(restored.deleted_at, None);
    let events = repositories
        .outbox
        .replay(0, chrono::DateTime::UNIX_EPOCH, 100)
        .await
        .unwrap();
    let metadata: Vec<_> = events
        .iter()
        .filter(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|e| e.event_type().starts_with("metadata_field."))
        })
        .collect();
    assert_eq!(
        metadata.len(),
        6,
        "no-op and rejected writes must not publish events"
    );
    assert!(metadata.iter().all(|event| event.event.is_none()));
    let created = metadata[0].envelope.as_ref().unwrap();
    assert_eq!(created.event_type(), "metadata_field.created");
    assert_eq!(created.vault_id(), vault);
    assert_eq!(created.subject().id, field.id);
    assert_eq!(created.actor_id(), field.created_by.as_ref());
    assert_eq!(created.document_id(), None);
    assert_eq!(
        created.audience(),
        &folio_core::domain_event::EventAudience::Vault
    );
    assert!(repo.list(vault, false, None, 101).await.is_err());

    let (first, second) = tokio::join!(
        repo.create(vault, input("Concurrent")),
        repo.create(vault, input("CONCURRENT")),
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "one concurrent name claim must win"
    );
    let page = repo.list(vault, false, None, 1).await.unwrap();
    let next = repo.list(vault, false, Some(page[0].id), 1).await.unwrap();
    assert_eq!(next.len(), 1);
    assert_ne!(page[0].id, next[0].id);

    match &db {
        Database::Sqlite(pool) => {
            sqlx::query("DROP TABLE outbox_events")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DROP TABLE outbox_events")
                .execute(pool)
                .await
                .unwrap();
        }
    }
    assert!(repo.create(vault, input("Rolled back")).await.is_err());
    assert_eq!(
        repo.list(vault, false, None, 10).await.unwrap().len(),
        2,
        "failed event publication must roll back the field"
    );
}

#[tokio::test]
async fn sqlite_fields_preserve_lifecycle_and_transactional_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("metadata.db").display()
    ))
    .await
    .unwrap();
    contract(db).await;
}

#[tokio::test]
async fn postgres_fields_preserve_lifecycle_and_transactional_events() {
    let Some(container) = common::container_or_skip(
        "metadata fields",
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
    contract(db).await;
}
