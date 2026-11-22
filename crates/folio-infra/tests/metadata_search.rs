mod common;

use folio_core::models::{
    CreateDocument, MetadataChange, MetadataExactFilter, MetadataFieldInput, MetadataKind,
    MetadataSearchQuery, MetadataValue,
};
use folio_core::ports::MetadataValueRepository;
use folio_core::Error;
use folio_infra::database::{
    postgres::PostgresMetadataValueRepository, sqlite::SqliteMetadataValueRepository,
};
use folio_infra::Database;

async fn search_contract(db: &Database) {
    let repos = db.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let other = repos.vaults.create("Other", None).await.unwrap().id;
    let values: Box<dyn MetadataValueRepository> = match db {
        Database::Sqlite(pool) => Box::new(SqliteMetadataValueRepository::new(pool.clone())),
        Database::Postgres(pool) => Box::new(PostgresMetadataValueRepository::new(pool.clone())),
    };
    let make_field = |name: &str, kind| MetadataFieldInput {
        name: name.into(),
        kind,
        required: false,
        unique: false,
    };
    let decimal = repos
        .metadata_fields
        .create(vault, make_field("Amount", MetadataKind::Decimal))
        .await
        .unwrap();
    let flag = repos
        .metadata_fields
        .create(vault, make_field("Paid", MetadataKind::Boolean))
        .await
        .unwrap();
    let foreign = repos
        .metadata_fields
        .create(other, make_field("Amount", MetadataKind::Decimal))
        .await
        .unwrap();

    let mut matches = Vec::new();
    for (name, owner, field, amount, paid) in [
        (
            "Invoice",
            vault,
            decimal.id,
            "9007199254740993.01",
            Some(false),
        ),
        ("Receipt", vault, decimal.id, "9007199254740993.0100", None),
        (
            "Nearest",
            vault,
            decimal.id,
            "9007199254740993.02",
            Some(true),
        ),
        ("Hidden", other, foreign.id, "9007199254740993.01", None),
    ] {
        let document = repos
            .documents
            .create(
                owner,
                CreateDocument {
                    title: name.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut changes = vec![MetadataChange {
            field_id: field,
            value: Some(MetadataValue::Decimal(amount.into())),
        }];
        if let Some(paid) = paid {
            changes.push(MetadataChange {
                field_id: flag.id,
                value: Some(MetadataValue::Boolean(paid)),
            });
        }
        values
            .patch(owner, document.id, None, changes)
            .await
            .unwrap();
        if matches!(name, "Invoice" | "Receipt") {
            matches.push(document.id);
        }
    }
    let trashed = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Trashed".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    values
        .patch(
            vault,
            trashed.id,
            None,
            vec![MetadataChange {
                field_id: decimal.id,
                value: Some(MetadataValue::Decimal("9007199254740993.01".into())),
            }],
        )
        .await
        .unwrap();
    repos.documents.delete(vault, trashed.id).await.unwrap();

    matches.sort();
    let query = |amount: &str, limit, cursor| MetadataSearchQuery {
        filter: MetadataExactFilter {
            field_id: decimal.id,
            value: MetadataValue::Decimal(amount.into()),
        },
        limit,
        cursor,
    };
    let first = values
        .search_exact(vault, query("9007199254740993.010", 1, None))
        .await
        .unwrap();
    assert_eq!(first.documents.len(), 1);
    assert_eq!(first.documents[0].id, matches[0]);
    let cursor = first.next_cursor.unwrap();
    let next = values
        .search_exact(vault, query("9007199254740993.01", 1, Some(cursor.clone())))
        .await
        .unwrap();
    assert_eq!(next.documents.len(), 1);
    assert_eq!(next.documents[0].id, matches[1]);
    assert!(next.next_cursor.is_none());
    assert!(matches!(
        values
            .search_exact(vault, query("9007199254740993.02", 1, Some(cursor)))
            .await,
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        values
            .search_exact(vault, query("9007199254740993.01", 101, None))
            .await,
        Err(Error::Validation(_))
    ));
    let boolean = values
        .search_exact(
            vault,
            MetadataSearchQuery {
                filter: MetadataExactFilter {
                    field_id: flag.id,
                    value: MetadataValue::Boolean(false),
                },
                limit: 10,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(boolean.documents.len(), 1);
    assert_eq!(boolean.documents[0].title, "Invoice");

    repos
        .metadata_fields
        .set_deleted(vault, decimal.id, true)
        .await
        .unwrap();
    assert!(matches!(
        values
            .search_exact(vault, query("9007199254740993.01", 1, None))
            .await,
        Err(Error::NotFound { .. })
    ));
    assert!(matches!(
        values
            .search_exact(other, query("9007199254740993.01", 1, None))
            .await,
        Err(Error::NotFound { .. })
    ));
}

#[tokio::test]
async fn sqlite_searches_exact_vault_values() {
    let (_dir, db) = common::sqlite_database().await;
    search_contract(&db).await;
}

#[tokio::test]
async fn postgres_searches_exact_vault_values() {
    let Some((_container, db)) = common::postgres_database("metadata exact search").await else {
        return;
    };
    search_contract(&db).await;
}
