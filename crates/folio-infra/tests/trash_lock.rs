use folio_core::models::CreateDocument;
use folio_infra::Database;

#[tokio::test]
async fn trash_waits_for_a_writer_before_capturing_its_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("trash.db").display()
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Before writer".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    let mut writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("UPDATE documents SET title = 'Committed writer' WHERE id = ?")
        .bind(document.id.to_string())
        .execute(&mut *writer)
        .await
        .unwrap();
    let documents = r.documents.clone();
    let mut trash = tokio::spawn(async move { documents.delete(vault, document.id).await });
    let early = tokio::time::timeout(std::time::Duration::from_millis(100), &mut trash).await;
    writer.commit().await.unwrap();
    assert!(
        early.is_err(),
        "trash must wait for the writer instead of failing a read-to-write upgrade: {early:?}"
    );
    trash.await.unwrap().unwrap();
    let payload: String =
        sqlx::query_scalar("SELECT payload FROM outbox_events WHERE type = 'trashed'")
            .fetch_one(pool)
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["envelope"]["data"]["title"], "Committed writer");
}
