mod common;

use folio_core::models::{FileIdentity, Id};
use folio_infra::Database;

async fn revision_history_contract(db: Database) {
    let vault = common::legacy_vault(&db).await;
    let other_vault = common::legacy_vault(&db).await;
    let repositories = db.repositories().await;
    let docs = repositories.documents;
    let document = docs
        .create_for_upload(vault, Default::default())
        .await
        .unwrap();
    let original = FileIdentity::from_bytes("original/file", "application/pdf", b"revision one");
    docs.finalize_verified_upload(vault, document.id, &original)
        .await
        .unwrap();

    let rows = docs
        .list_revisions(vault, document.id, None, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let first = &rows[0];
    assert_eq!(first.number, 1);
    assert!(first.is_current);
    assert_eq!(
        first.original.sha256.as_deref(),
        Some(original.sha256.as_str())
    );
    assert_eq!(first.original.byte_size, Some(original.byte_size));
    assert_eq!(
        docs.find_revision(vault, document.id, first.id)
            .await
            .unwrap(),
        Some(first.clone()),
    );

    docs.delete(vault, document.id).await.unwrap();
    assert!(docs
        .find_revision(vault, document.id, first.id)
        .await
        .unwrap()
        .is_none());
    docs.restore(vault, document.id).await.unwrap();
    assert_eq!(
        docs.find_revision(vault, document.id, first.id)
            .await
            .unwrap(),
        Some(first.clone())
    );

    let latest_id = Id::new_v4();
    let document_id = document.id;
    let sql = sqlx::AssertSqlSafe(format!(
        "INSERT INTO document_revisions
            (id, document_id, vault_id, number, original_file_id, created_at, restored_from_revision_id)
         SELECT '{latest_id}', document_id, vault_id, 2, original_file_id, created_at, id
         FROM document_revisions WHERE document_id = '{document_id}' AND number = 1;
         UPDATE documents SET current_revision_id = '{latest_id}' WHERE id = '{document_id}';"
    ));
    match &db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql(sql).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql(sql).execute(pool).await.unwrap();
        }
    };
    let latest = docs
        .list_revisions(vault, document.id, None, 1)
        .await
        .unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].id, latest_id);
    assert!(latest[0].is_current);
    assert_eq!(latest[0].restored_from_revision_id, Some(first.id));
    let older = docs
        .list_revisions(vault, document.id, Some(2), 1)
        .await
        .unwrap();
    let mut expected_first = first.clone();
    expected_first.is_current = false;
    assert_eq!(older, vec![expected_first]);

    assert!(docs
        .list_revisions(vault, document.id, Some(1), 10)
        .await
        .unwrap()
        .is_empty());
    assert!(docs
        .list_revisions(other_vault, document.id, None, 10)
        .await
        .unwrap()
        .is_empty());
    let other = docs.create(vault, Default::default()).await.unwrap();
    assert!(docs
        .find_revision(vault, other.id, first.id)
        .await
        .unwrap()
        .is_none());
    assert!(docs
        .find_revision(other_vault, document.id, first.id)
        .await
        .unwrap()
        .is_none());
    assert!(docs
        .find_revision(vault, document.id, Id::new_v4())
        .await
        .unwrap()
        .is_none());

    let sql = sqlx::AssertSqlSafe(format!(
        "UPDATE vaults SET deleted_at = '2001-02-03T04:05:07Z' WHERE id = '{vault}'"
    ));
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
    }
    assert!(docs
        .find_revision(vault, document.id, latest_id)
        .await
        .unwrap()
        .is_none());
    let sql = sqlx::AssertSqlSafe(format!(
        "UPDATE vaults SET deleted_at = NULL WHERE id = '{vault}'"
    ));
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
    }
    docs.delete(vault, document.id).await.unwrap();
    assert!(docs
        .list_revisions(vault, document.id, None, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(docs
        .find_revision(vault, document.id, first.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn sqlite_revision_reads_require_a_live_document_in_the_requested_vault() {
    let (_dir, db) = common::sqlite_database().await;
    revision_history_contract(db).await;
}

#[tokio::test]
async fn postgres_revision_reads_require_a_live_document_in_the_requested_vault() {
    let Some((_container, db)) = common::postgres_database("document revision history").await
    else {
        return;
    };
    revision_history_contract(db).await;
}
