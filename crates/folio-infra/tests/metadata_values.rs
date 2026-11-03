mod common;

use folio_core::models::{
    CreateDocument, Id, MetadataChange, MetadataFieldInput, MetadataKind, MetadataOptionInput,
    MetadataValue, UserId,
};
use folio_core::ports::{
    MetadataFieldRepository, MetadataOptionRepository, MetadataValueRepository,
};
use folio_core::Error;
use folio_infra::database::{
    postgres::{
        PostgresMetadataFieldRepository, PostgresMetadataOptionRepository,
        PostgresMetadataValueRepository,
    },
    sqlite::{
        SqliteMetadataFieldRepository, SqliteMetadataOptionRepository,
        SqliteMetadataValueRepository,
    },
};
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn field(name: &str, kind: MetadataKind, required: bool, unique: bool) -> MetadataFieldInput {
    MetadataFieldInput {
        name: name.into(),
        kind,
        required,
        unique,
    }
}

fn set(field_id: Id, value: MetadataValue) -> MetadataChange {
    MetadataChange {
        field_id,
        value: Some(value),
    }
}

fn clear(field_id: Id) -> MetadataChange {
    MetadataChange {
        field_id,
        value: None,
    }
}

fn document(title: &str) -> CreateDocument {
    CreateDocument {
        title: title.into(),
        ..Default::default()
    }
}

fn conflict<T: std::fmt::Debug>(result: folio_core::Result<T>) -> bool {
    matches!(result, Err(Error::Conflict(_)))
}

fn not_found<T: std::fmt::Debug>(result: folio_core::Result<T>) -> bool {
    matches!(result, Err(Error::NotFound { .. }))
}

async fn metadata_events(db: &Database) -> Vec<folio_core::DomainEventEnvelope> {
    db.repositories()
        .await
        .outbox
        .replay(0, chrono::DateTime::UNIX_EPOCH, 500)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|event| event.envelope)
        .filter(|event| event.event_type() == "document.metadata.changed")
        .collect()
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    let documents = &repositories.documents;
    let vault = repositories.vaults.instance_default().await.unwrap();
    let other_vault = repositories.vaults.create("Other", None).await.unwrap().id;
    let (fields, options, values): (
        Box<dyn MetadataFieldRepository>,
        Box<dyn MetadataOptionRepository>,
        Box<dyn MetadataValueRepository>,
    ) = match &db {
        Database::Sqlite(pool) => (
            Box::new(SqliteMetadataFieldRepository::new(pool.clone())),
            Box::new(SqliteMetadataOptionRepository::new(pool.clone())),
            Box::new(SqliteMetadataValueRepository::new(pool.clone())),
        ),
        Database::Postgres(pool) => (
            Box::new(PostgresMetadataFieldRepository::new(pool.clone())),
            Box::new(PostgresMetadataOptionRepository::new(pool.clone())),
            Box::new(PostgresMetadataValueRepository::new(pool.clone())),
        ),
    };
    let source = documents.create(vault, document("Source")).await.unwrap();
    let target = documents.create(vault, document("Target")).await.unwrap();
    let hidden = documents
        .create(other_vault, document("Hidden"))
        .await
        .unwrap();

    let reference = fields
        .create(vault, field("Reference", MetadataKind::Text, false, true))
        .await
        .unwrap();
    let paid = fields
        .create(vault, field("Paid", MetadataKind::Boolean, false, false))
        .await
        .unwrap();
    let amount = fields
        .create(vault, field("Amount", MetadataKind::Money, false, false))
        .await
        .unwrap();
    let count = fields
        .create(vault, field("Count", MetadataKind::Integer, true, false))
        .await
        .unwrap();
    let status = fields
        .create(vault, field("Status", MetadataKind::Select, false, false))
        .await
        .unwrap();
    let related = fields
        .create(
            vault,
            field("Related", MetadataKind::DocumentLink, false, false),
        )
        .await
        .unwrap();
    let open = options
        .create(
            vault,
            status.id,
            MetadataOptionInput {
                label: "Open".into(),
                position: 0,
            },
        )
        .await
        .unwrap();
    let foreign_field = fields
        .create(
            other_vault,
            field("Foreign", MetadataKind::Text, false, false),
        )
        .await
        .unwrap();

    let empty = values.get(vault, source.id).await.unwrap();
    assert_eq!(empty.revision, None);
    assert!(empty.values.is_empty());
    assert!(not_found(values.get(other_vault, source.id).await));

    let actor = UserId::from("value-editor");
    let first = folio_core::mutation::with_actor(
        Some(actor.clone()),
        values.patch(
            vault,
            source.id,
            None,
            vec![
                set(reference.id, MetadataValue::Text("INV-1".into())),
                set(paid.id, MetadataValue::Boolean(false)),
                set(
                    amount.id,
                    MetadataValue::Money {
                        amount: "0012.3400".into(),
                        currency: "EUR".into(),
                    },
                ),
                set(count.id, MetadataValue::Integer("-0042".into())),
                set(status.id, MetadataValue::Select(open.id)),
                set(related.id, MetadataValue::DocumentLink(target.id)),
            ],
        ),
    )
    .await
    .unwrap();
    let revision = first.revision.expect("a change assigns a revision");
    let value_of = |metadata: &folio_core::models::DocumentMetadata, field: Id| {
        metadata
            .values
            .iter()
            .find(|value| value.field_id == field)
            .and_then(|value| value.value.clone())
    };
    assert_eq!(
        value_of(&first, paid.id),
        Some(MetadataValue::Boolean(false)),
        "false must be stored, not treated as missing"
    );
    assert_eq!(
        value_of(&first, amount.id),
        Some(MetadataValue::Money {
            amount: "12.34".into(),
            currency: "EUR".into()
        })
    );
    assert_eq!(
        value_of(&first, count.id),
        Some(MetadataValue::Integer("-42".into()))
    );
    assert!(first
        .values
        .iter()
        .all(|value| value.created_by.as_ref() == Some(&actor)));
    assert_eq!(values.get(vault, source.id).await.unwrap(), first);

    let unchanged = values
        .patch(
            vault,
            source.id,
            Some(revision),
            vec![set(paid.id, MetadataValue::Boolean(false))],
        )
        .await
        .unwrap();
    assert_eq!(
        unchanged.revision,
        Some(revision),
        "unchanged values keep the revision"
    );
    assert!(conflict(
        values
            .patch(
                vault,
                source.id,
                None,
                vec![set(paid.id, MetadataValue::Boolean(true))]
            )
            .await
    ));

    for rejected in [
        vec![set(paid.id, MetadataValue::Text("yes".into()))],
        vec![clear(count.id)],
        vec![
            set(paid.id, MetadataValue::Boolean(true)),
            set(paid.id, MetadataValue::Boolean(false)),
        ],
        vec![
            set(paid.id, MetadataValue::Boolean(true)),
            set(
                amount.id,
                MetadataValue::Money {
                    amount: "1e3".into(),
                    currency: "EUR".into(),
                },
            ),
        ],
    ] {
        assert!(matches!(
            values
                .patch(vault, source.id, Some(revision), rejected)
                .await,
            Err(Error::Validation(_))
        ));
    }
    assert!(not_found(
        values
            .patch(
                vault,
                source.id,
                Some(revision),
                vec![set(foreign_field.id, MetadataValue::Text("x".into()))]
            )
            .await
    ));
    assert!(not_found(
        values
            .patch(
                vault,
                source.id,
                Some(revision),
                vec![set(related.id, MetadataValue::DocumentLink(hidden.id))]
            )
            .await
    ));
    assert!(not_found(
        values
            .patch(
                vault,
                source.id,
                Some(revision),
                vec![set(status.id, MetadataValue::Select(Id::new_v4()))]
            )
            .await
    ));
    assert_eq!(
        values.get(vault, source.id).await.unwrap(),
        first,
        "rejected patches change nothing"
    );

    let duplicate = documents
        .create(vault, document("Duplicate"))
        .await
        .unwrap();
    assert!(conflict(
        values
            .patch(
                vault,
                duplicate.id,
                None,
                vec![
                    set(paid.id, MetadataValue::Boolean(true)),
                    set(reference.id, MetadataValue::Text("INV-1".into())),
                ]
            )
            .await
    ));
    assert!(values
        .get(vault, duplicate.id)
        .await
        .unwrap()
        .values
        .is_empty());
    values
        .patch(
            vault,
            duplicate.id,
            None,
            vec![set(reference.id, MetadataValue::Text("inv-1".into()))],
        )
        .await
        .expect("text uniqueness is case-sensitive");
    assert!(conflict(
        fields
            .update(
                vault,
                reference.id,
                field("Reference", MetadataKind::LongText, false, true)
            )
            .await
    ));
    fields
        .update(
            vault,
            paid.id,
            field("Paid", MetadataKind::Boolean, false, true),
        )
        .await
        .expect("a single value cannot collide");
    values
        .patch(
            vault,
            duplicate.id,
            values.get(vault, duplicate.id).await.unwrap().revision,
            vec![set(paid.id, MetadataValue::Boolean(true))],
        )
        .await
        .unwrap();
    fields
        .update(
            vault,
            paid.id,
            field("Paid", MetadataKind::Boolean, false, false),
        )
        .await
        .unwrap();
    values
        .patch(
            vault,
            duplicate.id,
            values.get(vault, duplicate.id).await.unwrap().revision,
            vec![set(paid.id, MetadataValue::Boolean(false))],
        )
        .await
        .unwrap();
    assert!(
        conflict(
            fields
                .update(
                    vault,
                    paid.id,
                    field("Paid", MetadataKind::Boolean, false, true)
                )
                .await
        ),
        "enabling uniqueness must reject retained collisions"
    );

    let cleared = values
        .patch(vault, source.id, Some(revision), vec![clear(paid.id)])
        .await
        .unwrap();
    let tombstone = cleared
        .values
        .iter()
        .find(|value| value.field_id == paid.id)
        .unwrap();
    assert_eq!(tombstone.value, None);
    assert!(tombstone.deleted_at.is_some());
    assert_ne!(cleared.revision, Some(revision));

    fields.set_deleted(vault, status.id, true).await.unwrap();
    assert!(conflict(
        values
            .patch(
                vault,
                duplicate.id,
                values.get(vault, duplicate.id).await.unwrap().revision,
                vec![set(status.id, MetadataValue::Select(open.id))]
            )
            .await
    ));
    let retired_cleared = values
        .patch(vault, source.id, cleared.revision, vec![clear(status.id)])
        .await
        .expect("retired assignments can be cleared");
    fields.set_deleted(vault, status.id, false).await.unwrap();

    assert!(
        conflict(documents.move_to(source.id, vault, other_vault, None).await),
        "a document with values cannot move"
    );
    assert!(
        conflict(documents.move_to(target.id, vault, other_vault, None).await),
        "a link target cannot move"
    );

    documents.delete(vault, target.id).await.unwrap();
    assert!(conflict(
        values
            .patch(
                vault,
                duplicate.id,
                values.get(vault, duplicate.id).await.unwrap().revision,
                vec![set(related.id, MetadataValue::DocumentLink(target.id))]
            )
            .await
    ));
    assert_eq!(
        value_of(&values.get(vault, source.id).await.unwrap(), related.id),
        Some(MetadataValue::DocumentLink(target.id)),
        "trashing a target retains its incoming links"
    );
    documents.request_purge(vault, target.id).await.unwrap();
    documents.finalize_purge(vault, target.id).await.unwrap();
    let after_purge = values.get(vault, source.id).await.unwrap();
    let link = after_purge
        .values
        .iter()
        .find(|value| value.field_id == related.id)
        .unwrap();
    assert_eq!(link.value, None);
    assert_eq!(link.updated_by, None, "purge clears links as the system");
    assert_ne!(after_purge.revision, retired_cleared.revision);

    let mut remaining = vec![];
    for value in &after_purge.values {
        if value.value.is_some() && value.field_id != count.id {
            remaining.push(clear(value.field_id));
        }
    }
    fields
        .update(
            vault,
            count.id,
            field("Count", MetadataKind::Integer, false, false),
        )
        .await
        .unwrap();
    remaining.push(clear(count.id));
    values
        .patch(vault, source.id, after_purge.revision, remaining)
        .await
        .unwrap();
    let moved = documents
        .move_to(source.id, vault, other_vault, None)
        .await
        .expect("cleared values do not block a move");
    assert_eq!(moved.vault_id, other_vault);
    let in_new_vault = values.get(other_vault, source.id).await.unwrap();
    assert!(in_new_vault.values.is_empty());
    assert_eq!(in_new_vault.revision, None);

    let events = metadata_events(&db).await;
    let purge_event = events
        .iter()
        .find(|event| {
            event.data()["changes"].as_array().unwrap().len() == 1
                && event.data()["changes"][0]["field_id"] == serde_json::json!(related.id)
        })
        .expect("purge clearing emits a value-change event");
    assert_eq!(purge_event.document_id(), Some(source.id));
    assert_eq!(purge_event.actor_id(), None);
    assert_eq!(
        purge_event.data()["changes"][0]["before"]["value"],
        serde_json::json!(target.id)
    );
    let initial = &events[0];
    assert_eq!(initial.actor_id(), Some(&actor));
    assert_eq!(initial.vault_id(), vault);
    assert_eq!(initial.data()["changes"].as_array().unwrap().len(), 6);
    assert!(events.iter().all(|event| event.vault_id() == vault));

    let racer = documents.create(vault, document("Racer")).await.unwrap();
    let (a, b) = tokio::join!(
        values.patch(
            vault,
            racer.id,
            None,
            vec![set(reference.id, MetadataValue::Text("A".into()))]
        ),
        values.patch(
            vault,
            racer.id,
            None,
            vec![set(reference.id, MetadataValue::Text("B".into()))]
        ),
    );
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "concurrent edits on one revision conflict"
    );

    let rollback_events = metadata_events(&db).await.len();
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
    let before_rollback = values.get(vault, duplicate.id).await.unwrap();
    assert!(values
        .patch(
            vault,
            duplicate.id,
            before_rollback.revision,
            vec![set(paid.id, MetadataValue::Boolean(true))]
        )
        .await
        .is_err());
    assert_eq!(
        values.get(vault, duplicate.id).await.unwrap(),
        before_rollback
    );
    assert!(rollback_events > 0);
}

#[tokio::test]
async fn sqlite_values_are_typed_atomic_and_vault_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("values.db").display()
    ))
    .await
    .unwrap();
    contract(db).await;
}

#[tokio::test]
async fn postgres_values_are_typed_atomic_and_vault_scoped() {
    let Some(container) = common::container_or_skip(
        "metadata values",
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
