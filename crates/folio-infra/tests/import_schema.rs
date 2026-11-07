mod common;

use folio_core::models::{CreateDocument, Id};
use folio_infra::Database;

async fn set_external_ref(
    db: &Database,
    id: Id,
    source: Option<&str>,
    external_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let sql = "UPDATE documents SET external_source = $1, external_id = $2 WHERE id = $3";
    match db {
        Database::Sqlite(pool) => sqlx::query(sql)
            .bind(source)
            .bind(external_id)
            .bind(id.to_string())
            .execute(pool)
            .await
            .map(|_| ()),
        Database::Postgres(pool) => sqlx::query(sql)
            .bind(source)
            .bind(external_id)
            .bind(id)
            .execute(pool)
            .await
            .map(|_| ()),
    }
}

async fn contract(db: Database) {
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let create = |title: &str| CreateDocument {
        title: title.into(),
        ..Default::default()
    };
    let first = r.documents.create(vault, create("first")).await.unwrap();
    let second = r.documents.create(vault, create("second")).await.unwrap();

    set_external_ref(&db, first.id, Some("paperless"), Some("7"))
        .await
        .unwrap();
    r.documents.delete(vault, first.id).await.unwrap();
    assert!(
        set_external_ref(&db, second.id, Some("paperless"), Some("7"))
            .await
            .is_err(),
        "a trashed document still owns its external ref"
    );
    set_external_ref(&db, second.id, Some("paperless"), Some("8"))
        .await
        .unwrap();

    let third = r.documents.create(vault, create("third")).await.unwrap();
    assert!(set_external_ref(&db, third.id, None, Some("9"))
        .await
        .is_err());
    assert!(set_external_ref(&db, third.id, Some("paperless"), None)
        .await
        .is_err());

    let other = r.vaults.create("Other", None).await.unwrap();
    let elsewhere = r.documents.create(other.id, create("other")).await.unwrap();
    set_external_ref(&db, elsewhere.id, Some("paperless"), Some("7"))
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_external_refs_are_unique_per_vault_including_trash() {
    let (_dir, db) = common::sqlite_database().await;
    contract(db).await;
}

#[tokio::test]
async fn postgres_external_refs_are_unique_per_vault_including_trash() {
    let Some((_container, db)) = common::postgres_database("import schema").await else {
        return;
    };
    contract(db).await;
}
