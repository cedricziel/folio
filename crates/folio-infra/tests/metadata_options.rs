mod common;

use folio_core::models::{MetadataFieldInput, MetadataKind, MetadataOptionInput, UserId, VaultId};
use folio_core::ports::{MetadataFieldRepository, MetadataOptionRepository};
use folio_infra::database::{
    postgres::{PostgresMetadataFieldRepository, PostgresMetadataOptionRepository},
    sqlite::{SqliteMetadataFieldRepository, SqliteMetadataOptionRepository},
};
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn input(label: &str, position: u32) -> MetadataOptionInput {
    MetadataOptionInput {
        label: label.into(),
        position,
    }
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    let vault = repositories.vaults.instance_default().await.unwrap();
    let (fields, options): (
        Box<dyn MetadataFieldRepository>,
        Box<dyn MetadataOptionRepository>,
    ) = match &db {
        Database::Sqlite(pool) => (
            Box::new(SqliteMetadataFieldRepository::new(pool.clone())),
            Box::new(SqliteMetadataOptionRepository::new(pool.clone())),
        ),
        Database::Postgres(pool) => (
            Box::new(PostgresMetadataFieldRepository::new(pool.clone())),
            Box::new(PostgresMetadataOptionRepository::new(pool.clone())),
        ),
    };
    let field = fields
        .create(
            vault,
            MetadataFieldInput {
                name: "Status".into(),
                kind: MetadataKind::Select,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    let text_field = fields
        .create(
            vault,
            MetadataFieldInput {
                name: "Reference".into(),
                kind: MetadataKind::Text,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    assert!(options
        .create(vault, text_field.id, input("Invalid", 0))
        .await
        .is_err());
    assert!(options
        .create(VaultId::new(), field.id, input("Hidden", 0))
        .await
        .is_err());
    let actor = UserId::from("option-editor");
    let original = folio_core::mutation::with_actor(
        Some(actor.clone()),
        options.create(vault, field.id, input(" Pending ", u32::MAX)),
    )
    .await
    .unwrap();
    assert_eq!(original.label, "Pending");
    assert_eq!(original.position, u32::MAX);
    assert_eq!(original.created_by, Some(actor));
    assert_eq!(original.field_id, field.id);
    assert!(options
        .create(vault, field.id, input("PENDING", 0))
        .await
        .is_err());
    assert!(options
        .find(VaultId::new(), field.id, original.id)
        .await
        .unwrap()
        .is_none());
    assert!(options
        .find(vault, uuid::Uuid::new_v4(), original.id)
        .await
        .unwrap()
        .is_none());
    let unchanged = options
        .update(vault, field.id, original.id, input("Pending", u32::MAX))
        .await
        .unwrap();
    assert_eq!(unchanged.updated_at, original.updated_at);
    let renamed = options
        .update(vault, field.id, original.id, input("Waiting", 1))
        .await
        .unwrap();
    assert_eq!(renamed.id, original.id);
    assert_eq!(renamed.created_by, original.created_by);
    assert_eq!(renamed.created_at, original.created_at);
    assert_eq!(renamed.position, 1);
    let retired = options
        .set_deleted(vault, field.id, original.id, true)
        .await
        .unwrap();
    assert!(retired.deleted_at.is_some());
    assert_eq!(
        options
            .set_deleted(vault, field.id, original.id, true)
            .await
            .unwrap()
            .updated_at,
        retired.updated_at
    );
    assert!(options
        .update(vault, field.id, original.id, input("Changed", 0))
        .await
        .is_err());
    let replacement = options
        .create(vault, field.id, input("Waiting", 0))
        .await
        .unwrap();
    assert!(options
        .set_deleted(vault, field.id, original.id, false)
        .await
        .is_err());
    options
        .set_deleted(vault, field.id, replacement.id, true)
        .await
        .unwrap();
    assert!(options
        .set_deleted(vault, field.id, original.id, false)
        .await
        .unwrap()
        .deleted_at
        .is_none());
    assert_eq!(
        options
            .list(vault, field.id, false, None, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        options
            .list(vault, field.id, true, None, 100)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(options
        .list(vault, field.id, false, None, 101)
        .await
        .is_err());
    assert!(
        fields
            .update(
                vault,
                field.id,
                MetadataFieldInput {
                    name: "Status".into(),
                    kind: MetadataKind::Text,
                    required: false,
                    unique: false,
                }
            )
            .await
            .is_err(),
        "retained options must prevent changing their field kind"
    );
    fields.set_deleted(vault, field.id, true).await.unwrap();
    assert!(options
        .create(vault, field.id, input("Blocked", 0))
        .await
        .is_err());
    assert!(options
        .update(vault, field.id, original.id, input("Blocked", 0))
        .await
        .is_err());
    fields.set_deleted(vault, field.id, false).await.unwrap();
    let events = repositories
        .outbox
        .replay(0, chrono::DateTime::UNIX_EPOCH, 100)
        .await
        .unwrap();
    let events: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| event.event_type().starts_with("metadata_option."))
        .collect();
    assert_eq!(
        events.len(),
        6,
        "rejected changes and no-ops must not emit option events"
    );
    assert_eq!(events[0].subject().id, original.id);
    assert_eq!(events[0].document_id(), None);
    assert_eq!(events[0].vault_id(), vault);
    assert_eq!(events[0].actor_id(), original.created_by.as_ref());
    assert_eq!(
        events[0].audience(),
        &folio_core::domain_event::EventAudience::Vault
    );
    let (first, second) = tokio::join!(
        options.create(vault, field.id, input("Concurrent", 0)),
        options.create(vault, field.id, input("CONCURRENT", 0)),
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let page = options.list(vault, field.id, false, None, 1).await.unwrap();
    let next = options
        .list(vault, field.id, false, Some(page[0].id), 1)
        .await
        .unwrap();
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
    assert!(options
        .create(vault, field.id, input("Rolled back", 0))
        .await
        .is_err());
    assert_eq!(
        options
            .list(vault, field.id, false, None, 100)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn sqlite_options_preserve_identity_and_transactional_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("options.db").display()
    ))
    .await
    .unwrap();
    contract(db).await;
}

#[tokio::test]
async fn postgres_options_preserve_identity_and_transactional_events() {
    let Some(container) = common::container_or_skip(
        "metadata options",
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
